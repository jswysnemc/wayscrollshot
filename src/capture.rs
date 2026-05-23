use std::io::{self, Read};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;

use crate::types::Region;

/// Prompts the user to select a capture region with `slurp`.
pub fn select_region() -> Result<Region> {
    let output = Command::new("slurp")
        .arg("-f")
        .arg("%x,%y %wx%h")
        .output()
        .context("failed to run slurp")?;
    if !output.status.success() {
        bail!("slurp exited with non-zero status");
    }
    let raw = String::from_utf8(output.stdout)?.trim().to_string();
    if raw.is_empty() {
        bail!("slurp returned empty selection");
    }
    log::debug!("slurp output: {}", raw);
    region_from_slurp_output(&raw)
}

/// Parses an existing `slurp` geometry output.
pub fn region_from_slurp_output(raw: &str) -> Result<Region> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("slurp output is empty");
    }
    log::debug!("using slurp output: {}", raw);
    parse_region(raw)
}

/// Reads `slurp` geometry output from stdin.
pub fn read_region_from_stdin() -> Result<Region> {
    let mut raw = String::new();
    io::stdin()
        .read_to_string(&mut raw)
        .context("failed to read slurp output from stdin")?;
    region_from_slurp_output(&raw)
}

fn parse_region(raw: &str) -> Result<Region> {
    let mut parts = raw.split_whitespace();
    let coords = parts.next().ok_or_else(|| anyhow!("missing coords"))?;
    let size = parts.next().ok_or_else(|| anyhow!("missing size"))?;
    let (x_str, y_str) = coords
        .split_once(',')
        .ok_or_else(|| anyhow!("invalid coords"))?;
    let (w_str, h_str) = size
        .split_once('x')
        .ok_or_else(|| anyhow!("invalid size"))?;
    let x: i32 = x_str.parse()?;
    let y: i32 = y_str.parse()?;
    let w: u32 = w_str.parse()?;
    let h: u32 = h_str.parse()?;
    Ok(Region {
        raw: format!("{x},{y} {w}x{h}"),
        x,
        y,
        w,
        h,
    })
}

/// Captures a PNG frame from the selected region via `grim`.
pub fn capture_frame(region: &Region) -> Result<RgbaImage> {
    log::debug!("grim capture region: {}", region.raw);
    let output = Command::new("grim")
        .arg("-g")
        .arg(&region.raw)
        .arg("-t")
        .arg("png")
        .arg("-l")
        .arg("0")
        .arg("-s")
        .arg("1")
        .arg("-")
        .output()
        .context("failed to run grim")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::error!("grim stderr: {}", stderr);
        bail!("grim exited with non-zero status");
    }
    let image = image::load_from_memory(&output.stdout)?;
    Ok(image.to_rgba8())
}

#[cfg(test)]
mod tests {
    use super::region_from_slurp_output;

    #[test]
    fn parses_slurp_geometry() {
        let region = region_from_slurp_output("10,20 300x400").unwrap();

        assert_eq!(region.raw, "10,20 300x400");
        assert_eq!(region.x, 10);
        assert_eq!(region.y, 20);
        assert_eq!(region.w, 300);
        assert_eq!(region.h, 400);
    }

    #[test]
    fn parses_negative_coordinates() {
        let region = region_from_slurp_output("-1920,-10 800x600").unwrap();

        assert_eq!(region.raw, "-1920,-10 800x600");
        assert_eq!(region.x, -1920);
        assert_eq!(region.y, -10);
        assert_eq!(region.w, 800);
        assert_eq!(region.h, 600);
    }

    #[test]
    fn normalizes_whitespace() {
        let region = region_from_slurp_output("  10,20\t300x400\n").unwrap();

        assert_eq!(region.raw, "10,20 300x400");
    }

    #[test]
    fn rejects_empty_output() {
        assert!(region_from_slurp_output("\n").is_err());
    }
}
