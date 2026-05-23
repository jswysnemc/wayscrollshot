# wayscrollshot

A scrolling screenshot tool for Wayland that captures and stitches images in real-time as you scroll.
![preview](./preview.png)

[中文文档](README_CN.md)

## Features

- Real-time preview with automatic stitching
- Column sampling algorithm for fast and accurate overlap detection
- Rounded button UI with hover effects (powered by tiny-skia)
- Keyboard shortcuts and mouse control
- Save to file or copy to clipboard
- Supports reverse scrolling (col-sample only)

## How It Works

### Architecture

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│   Capture   │────>│   Stitcher   │────>│   Preview   │
│   (grim)    │     │ (col-sample) │     │ (layer-shell)│
└─────────────┘     └──────────────┘     └─────────────┘
```

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
| `grim` | Screen capture | Yes |
| OpenCV | Default stitching algorithm | Yes |
| `wl-copy` | Clipboard (Wayland) | For clipboard feature |
| `xclip` | Clipboard (X11 fallback) | Alternative |

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

### From Source

```bash
# Install runtime and build dependencies (Arch Linux)
sudo pacman -S slurp grim wl-clipboard opencv clang libxkbcommon

# Build
cargo build --release

# Install (optional)
cp target/release/wayscrollshot ~/.local/bin/
```

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
wayscrollshot -a opencv-orb  # Default: ORB feature matching + RANSAC
wayscrollshot -a col-sample  # Fast column sampling
wayscrollshot -a template    # Template matching (more accurate)
wayscrollshot -a edge        # Edge detection (for transparent backgrounds)
wayscrollshot -a fast        # FAST corner + HNSW index (experimental)
```

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `-o, --output <PATH>` | Output file path | `~/Pictures/wayscrollshot/wayscrollshot-<timestamp>.png` |
| `-w, --preview-width <PX>` | Preview width in pixels | 280 |
| `-c, --clipboard` | Copy to clipboard instead of saving | false |
| `--no-preview` | Disable preview window | false |
| `--no-border` | Disable region border overlay | false |
| `-a, --algorithm <ALG>` | Stitching algorithm: `opencv-orb`, `col-sample`, `template`, `edge`, `fast` | opencv-orb |
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

## Limitations

1. **Wayland only**: X11 is not supported. The tool uses `wlr-layer-shell-unstable-v1` protocol.

2. **wlroots-based compositors**: Works on Sway, Hyprland, river, etc. May not work on GNOME/KDE Wayland.

3. **Overlap requirement**: Each scroll step must leave some overlap with the previous view. Very fast scrolling may cause stitching failures.

4. **Static content assumption**: The algorithm assumes the scrolling content is static. Dynamic content (animations, videos) will cause artifacts.

5. **Vertical scrolling only**: Horizontal scrolling is not currently supported.

6. **Fixed header/footer**: If the page has fixed headers or footers, they will be captured repeatedly. Consider selecting a region that excludes them.

## Troubleshooting

### "slurp selection failed"
- Ensure `slurp` is installed and in PATH
- Check if you're running on Wayland

### "layer-shell not available"
- Your compositor doesn't support `wlr-layer-shell-unstable-v1`
- Try a wlroots-based compositor (Sway, Hyprland)

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
