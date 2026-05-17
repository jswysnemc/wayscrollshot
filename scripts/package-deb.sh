#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
    echo "usage: $0 <binary> <output-dir> [debian-revision-suffix]" >&2
    exit 2
fi

binary_path="$(realpath "$1")"
output_dir="$(realpath -m "$2")"
revision_suffix="${3:-}"

if [[ ! -x "$binary_path" ]]; then
    echo "binary is not executable: $binary_path" >&2
    exit 2
fi

for tool in cut date dpkg dpkg-deb dpkg-shlibdeps du gzip install mktemp realpath sed tr; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "required tool not found: $tool" >&2
        exit 2
    fi
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

pkgname="$(sed -n 's/^name = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"

if [[ -z "$pkgname" || -z "$version" ]]; then
    echo "failed to read package name or version from Cargo.toml" >&2
    exit 2
fi

if [[ -z "$revision_suffix" && -r /etc/os-release ]]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    revision_suffix="${ID:-linux}${VERSION_ID:-}"
fi

revision_suffix="$(printf '%s' "$revision_suffix" | tr -cd 'A-Za-z0-9.+~')"
deb_version="${version}-1${revision_suffix}"
arch="$(dpkg --print-architecture)"

mkdir -p "$output_dir"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

pkgroot="$tmpdir/pkg"
install -Dm755 "$binary_path" "$pkgroot/usr/bin/$pkgname"
install -Dm644 LICENSE "$pkgroot/usr/share/doc/$pkgname/copyright"

changelog="$tmpdir/changelog.Debian"
cat >"$changelog" <<EOF
$pkgname ($deb_version) unstable; urgency=medium

  * Release $version.

 -- Snemc-s <snemc@snemc.cn>  $(date -R)
EOF
gzip -n -9 -c "$changelog" >"$pkgroot/usr/share/doc/$pkgname/changelog.Debian.gz"

mkdir -p "$tmpdir/debian"
cat >"$tmpdir/debian/control" <<EOF
Source: $pkgname
Section: utils
Priority: optional
Maintainer: Snemc-s <snemc@snemc.cn>
Build-Depends: debhelper-compat (= 13)
Standards-Version: 4.7.0
Homepage: https://github.com/jswysnemc/wayscrollshot

Package: $pkgname
Architecture: any
Depends: \${shlibs:Depends}, \${misc:Depends}
Description: Scrolling screenshot tool for Wayland
 wayscrollshot captures a Wayland screen region and stitches scrolling frames.
EOF

shlibs_output="$(cd "$tmpdir" && dpkg-shlibdeps -O "$pkgroot/usr/bin/$pkgname")"
auto_depends="$(printf '%s\n' "$shlibs_output" | sed -n 's/^shlibs:Depends=//p')"
manual_depends="grim, slurp"

if [[ -n "$auto_depends" ]]; then
    depends="$auto_depends, $manual_depends"
else
    depends="$manual_depends"
fi

installed_size="$(du -sk "$pkgroot/usr" | cut -f1)"
mkdir -p "$pkgroot/DEBIAN"
cat >"$pkgroot/DEBIAN/control" <<EOF
Package: $pkgname
Version: $deb_version
Section: utils
Priority: optional
Architecture: $arch
Installed-Size: $installed_size
Maintainer: Snemc-s <snemc@snemc.cn>
Homepage: https://github.com/jswysnemc/wayscrollshot
Depends: $depends
Recommends: wl-clipboard | xclip
Description: Scrolling screenshot tool for Wayland
 wayscrollshot captures a Wayland screen region and stitches scrolling frames.
EOF

deb_path="$output_dir/${pkgname}_${deb_version}_${arch}.deb"
dpkg-deb --build --root-owner-group "$pkgroot" "$deb_path"
echo "$deb_path"
