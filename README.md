# wayscrollshot

A scrolling screenshot tool for Wayland that captures and stitches images in real-time as you scroll.
![preview](./preview.png)

[中文文档](README_CN.md)

## Features

- Real-time preview with automatic stitching
- Eight-band stitch that ignores static sidebars and fixed header/footer bars
- Rounded button UI with hover effects (powered by tiny-skia)
- Keyboard shortcuts and mouse control
- Save to file or copy to clipboard
- Supports reverse scrolling (`band` and col-sample)

## How It Works

### Architecture

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│   Capture   │────>│   Stitcher   │────>│   Preview   │
│   (grim)    │     │    (band)    │     │ (layer-shell)│
└─────────────┘     └──────────────┘     └─────────────┘
```

### Default stitch (`band`)

The default algorithm is the eight-band row signature used by mark-shot:

1. Sample eight narrow vertical bands and build one brightness signature per row.
2. Compare the new frame with the previous frame at the same rows. Bands that barely change are treated as a static sidebar and dropped from the match.
3. Search for the scroll offset on the moving bands, then append only the new rows.
4. Rows that stay identical at the top or bottom are treated as a fixed header or footer and are not repeated.

`-a col-sample` still uses the older column sampler described below.

### Column Sampling Algorithm

Instead of comparing entire images pixel-by-pixel, wayscrollshot uses a column sampling approach inspired by [screenshot-splicing](https://github.com/baotlake/screenshot-splicing):

1. **Sample 3 column groups** from each frame:
   - Left region (20 to width/4)
   - Middle region (width/2 to 5\*width/8)
   - Right region (6\*width/8 to 7\*width/8)

2. **Convert to grayscale** and average each group

3. **Search for overlap** using Mean Absolute Difference (MAD):
   - Start from the predicted offset (based on previous scroll)
   - Expand search outward: `[p, p+1, p-1, p+2, p-2, ...]`
   - Early termination when MAD < threshold

4. **Append new content** to the stitched image

**Complexity**: O(9 \* height) instead of O(width \* height) - a significant speedup.

### Overlap Detection

```
Frame 1 (previous):          Frame 2 (current):
┌────────────────┐           ┌────────────────┐
│    Content A   │           │    Content B   │
│                │           │                │
│    Content B   │ <──────── │    Content B   │  (overlap)
│                │           │                │
│    Content C   │           │    Content C   │
└────────────────┘           │                │
                             │    Content D   │  (new)
                             └────────────────┘
```

The algorithm finds where Frame 2's top matches Frame 1's content, then appends only the new portion.

## Dependencies

### Runtime Dependencies

| Tool / Library | Purpose | Required |
|----------------|---------|----------|
| `slurp` | Region selection | Unless `REGION` is provided |
| `grim` | Screen capture on wlroots compositors | wlroots only (not used on KDE) |
| OpenCV | Default stitching algorithm | Yes |
| `wl-copy` | Clipboard (Wayland) | For clipboard feature |
| `xclip` | Clipboard (X11 fallback) | Alternative |

On KDE Plasma, capture goes through the built-in `org.kde.KWin.ScreenShot2` D-Bus interface instead of `grim`; see [Compositor Compatibility](#compositor-compatibility).

### Build Dependencies

| Crate | Purpose |
|-------|---------|
| `smithay-client-toolkit` | Wayland client library |
| `wayland-client` | Wayland protocol bindings |
| `tiny-skia` | 2D graphics (rounded buttons) |
| `image` | Image processing and resizing |
| `opencv` | ORB feature matching and RANSAC alignment |
| `clap` | Command-line argument parsing |
| `anyhow` | Error handling |
| `chrono` | Timestamp for filenames |
| `log` / `env_logger` | Logging |

OpenCV development files and Clang are required when building from source.

## Installation

### Debian / Ubuntu

Download the `.deb` package that matches your Ubuntu release and CPU architecture from GitHub Releases, then install it with `apt`:

```bash
sudo apt install ./wayscrollshot_0.1.7-1ubuntu26.04_amd64.deb
```

The Debian packages are built separately for each Ubuntu release so OpenCV runtime dependencies match that release's ABI. Do not work around missing OpenCV libraries by symlinking a different OpenCV version.

`apt install ./wayscrollshot_….deb` pulls `grim`, `slurp`, and the OpenCV shared libraries recorded in the package. Do not run `apt install opencv clang`: those names are for Arch, and Ubuntu has no package called `opencv`. If the `.deb` was installed with `dpkg -i` and libraries are missing, run `sudo apt install -f`.

GNOME needs the bundled Shell extension (Mutter has no layer-shell, so `slurp` and `grim` cannot run there). After installing the package, enable it and log out once:

```bash
gnome-extensions enable wayscrollshot-scroll-helper@snemc.org
```

### From Source

```bash
# Arch Linux
sudo pacman -S slurp grim wl-clipboard opencv clang libxkbcommon

# Debian / Ubuntu / Pop!_OS (package names differ from Arch)
sudo apt install build-essential pkg-config clang libclang-dev libopencv-dev libxkbcommon-dev slurp grim wl-clipboard

# Build
cargo build --release

# Install (optional)
cp target/release/wayscrollshot ~/.local/bin/
```

On Ubuntu the OpenCV development package is `libopencv-dev`, not `opencv`. `clang` is only required to compile; the installed `.deb` does not need it.

### From Nix

#### Run

```shell
nix run github:jswysnemc/wayscrollshot
```

#### home-manager configuration

```nix
# input
inputs = {
  nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  
  wayscrollshot = {
    url = "github:jswysnemc/wayscrollshot";
    inputs.nixpkgs.follows = "nixpkgs";
  };
}
```

## Usage

```bash
# Basic usage
wayscrollshot

# Save to specific file
wayscrollshot -o ~/screenshot.png

# Copy to clipboard instead of saving
wayscrollshot -c

# Custom preview width
wayscrollshot -w 320

# Disable preview window
wayscrollshot --no-preview

# Disable region border overlay
wayscrollshot --no-border

# Use an existing slurp selection
wayscrollshot "$(slurp)"

# Read slurp output from stdin
slurp | wayscrollshot -

# Use different stitching algorithms
wayscrollshot -a band        # Default: eight-band match, ignores static sidebars and fixed bars
wayscrollshot -a opencv-orb  # ORB feature matching + RANSAC
wayscrollshot -a col-sample  # Fast column sampling
wayscrollshot -a template    # Template matching (more accurate)
wayscrollshot -a edge        # Edge detection (for transparent backgrounds)
wayscrollshot -a fast        # FAST corner + HNSW index (experimental)

# Select capture backend (auto-detected by default)
wayscrollshot --backend grim   # wlroots: Sway, Hyprland, niri, river
wayscrollshot --backend kwin   # KDE Plasma (KWin ScreenShot2)
wayscrollshot --backend gnome  # GNOME Shell helper extension
```

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `-o, --output <PATH>` | Output file path | `~/Pictures/wayscrollshot/wayscrollshot-<timestamp>.png` |
| `-w, --preview-width <PX>` | Preview width in pixels | 280 |
| `-c, --clipboard` | Copy to clipboard instead of saving | false |
| `--no-preview` | Disable preview window | false |
| `--no-border` | Disable region border overlay | false |
| `-a, --algorithm <ALG>` | Stitching algorithm: `band`, `opencv-orb`, `col-sample`, `template`, `edge`, `fast` | band |
| `--backend <BACKEND>` | Capture backend: `auto`, `grim`, `kwin`, `gnome` | auto |
| `REGION` | Existing slurp/grim geometry, for example `10,20 300x400`; use `-` to read stdin | Run `slurp` interactively |

### Controls

**Mouse:**
- Click buttons in the control bar

**Control bar buttons:**
| Button | Action |
|--------|--------|
| `S` (Save) | Save image and exit |
| `C` (Copy) | Copy to clipboard and exit |
| `P` (Pause/Resume) | Pause or resume capture |
| `X` (Cancel) | Cancel capture and exit |

**Keyboard (when overlay is focused):**
| Key | Action |
|-----|--------|
| `S` | Save and exit |
| `C` | Copy to clipboard and exit |
| `Space` | Pause/Resume capture |
| `Q` / `Esc` | Cancel and exit |

> **Note:** Keyboard shortcuts depend on the compositor's `wlr-layer-shell` `OnDemand` keyboard focus policy. They work well under niri; on Hyprland and some other compositors you may need to hover the mouse over the control bar first. Mouse clicks on the control bar buttons work on all compositors.

## Compositor Compatibility

The preview overlay and region border use `wlr-layer-shell-unstable-v1`, region selection uses `slurp`, and screen capture uses a backend chosen at runtime (override with `--backend`).

| Desktop | Region select | Preview / border | Capture backend | Status |
|---------|---------------|------------------|-----------------|--------|
| wlroots (Sway, Hyprland, niri, river) | `slurp` | layer-shell | `grim` | Works out of the box |
| KDE Plasma (KWin) | `slurp` | layer-shell | KWin ScreenShot2 (D-Bus) | Works; needs the authorization step below |
| GNOME (Mutter) | Shell helper | Shell helper buttons | Shell helper `ScreenshotArea` | Needs `wayscrollshot-scroll-helper@snemc.org` enabled, then a logout |

### KDE authorization

KWin only lets a process call `org.kde.KWin.ScreenShot2` when that process owns a `.desktop` entry declaring `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2`. The deb, AUR, and Nix packages install `wayscrollshot.desktop` for this. When running a self-built binary, install that file into `/usr/share/applications/` (or `~/.local/share/applications/`), or bypass the check for a quick test:

```bash
KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1 wayscrollshot
```

## Limitations

1. **Wayland only**: X11 is not supported. The preview and region border use `wlr-layer-shell-unstable-v1` (supported by wlroots compositors and KWin, not by GNOME).

2. **Desktop support**: See [Compositor Compatibility](#compositor-compatibility). KDE Plasma works through KWin ScreenShot2. GNOME works through the Shell helper extension; the preview overlay is replaced by Save / Copy / Pause / Cancel buttons.

3. **Overlap requirement**: Each scroll step must leave some overlap with the previous view. Very fast scrolling may cause stitching failures.

4. **Static content assumption**: The algorithm assumes the scrolling content is static. Dynamic content (animations, videos) will cause artifacts.

5. **Vertical scrolling only**: Horizontal scrolling is not currently supported.

6. **Fixed chrome**: The default `band` algorithm drops stable headers, footers, and sidebars that do not scroll with the page. A sidebar whose highlight moves much less than the article is treated as static. Select a tighter region if a bar still duplicates.

## Troubleshooting

### "Unable to locate package opencv" (or `clang`)
- That command is the Arch package list. On Ubuntu install `libopencv-dev` only when building from source, and install a release `.deb` with `sudo apt install ./wayscrollshot_….deb` when you only want to run it.
- `grim` and `slurp` are in the Ubuntu `universe` component.

### "slurp selection failed"
- On GNOME this is expected: enable `wayscrollshot-scroll-helper@snemc.org` and log out once. `slurp` needs `wlr-layer-shell`, which Mutter does not implement.
- On wlroots and KDE, ensure `slurp` is installed and in PATH
- Check if you're running on Wayland

### "layer-shell not available"
- Your compositor doesn't support `wlr-layer-shell-unstable-v1`
- Try a wlroots-based compositor (Sway, Hyprland) or KDE Plasma

### "KWin denied the screenshot (NoAuthorized)" (KDE)
- Install `wayscrollshot.desktop` (shipped by the packages) into a standard applications directory; it must declare `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2`
- For a quick test without installing it, run with `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1`

### "No overlap match"
- Scroll more slowly
- Ensure there's visible overlap between frames
- Avoid scrolling through completely different content

### Preview not updating
- Check if the capture region is correct
- Try running with `RUST_LOG=debug` for more info

## License

MIT

## Contributing

Contributions are welcome! Areas that could use improvement:

- **Algorithm optimization**: The `fast` algorithm (FAST corner + HNSW) needs tuning for better accuracy
- **Cross-platform support**: Currently Linux-only due to Wayland dependency
- **Performance**: Reduce memory usage for very long screenshots
- **UI improvements**: Better visual feedback during capture

Please open an issue to discuss major changes before submitting a PR.

## Acknowledgments

- [screenshot-splicing](https://github.com/baotlake/screenshot-splicing) - Column sampling algorithm inspiration
- [snow-shot](https://github.com/mg-chao/snow-shot) - FAST corner + HNSW algorithm reference
- [smithay-client-toolkit](https://github.com/Smithay/client-toolkit) - Wayland client library
- [tiny-skia](https://github.com/RazrFalcon/tiny-skia) - 2D graphics library

- Friend Links[Linux.do社区](https://linux.do/)
