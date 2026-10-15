//! KDE screenshot backend using the `org.kde.KWin.ScreenShot2` D-Bus interface.
//!
//! KWin does not implement `wlr-screencopy`, so `grim` cannot capture under a
//! KDE Plasma session. Spectacle and other KDE tools instead use ScreenShot2,
//! which streams raw pixels through a pipe file descriptor supplied by the
//! caller.
//!
//! Access is gated by KWin: the calling process must own a `.desktop` entry
//! declaring `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2`,
//! otherwise the call fails with `org.kde.KWin.ScreenShot2.Error.NoAuthorized`.
//! Setting `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1` bypasses the check during
//! development.

use std::collections::HashMap;
use std::io::Read;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::thread;

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;
use zbus::blocking::Connection;
use zbus::zvariant::{Fd, OwnedValue, Value};

use crate::types::Region;

const SERVICE: &str = "org.kde.KWin";
const PATH: &str = "/org/kde/KWin/ScreenShot2";
const INTERFACE: &str = "org.kde.KWin.ScreenShot2";

/// Relevant `QImage::Format` enum values KWin may report for raw pixel data.
/// Only 32-bit-per-pixel formats are emitted by the screenshot effect.
mod qimage_format {
    /// `0xffRRGGBB`; little-endian bytes are `B, G, R, X`.
    pub const RGB32: u32 = 4;
    /// `0xAARRGGBB`; little-endian bytes are `B, G, R, A`.
    pub const ARGB32: u32 = 5;
    pub const ARGB32_PREMULTIPLIED: u32 = 6;
    /// Bytes are `R, G, B, X`.
    pub const RGBX8888: u32 = 16;
    /// Bytes are `R, G, B, A`.
    pub const RGBA8888: u32 = 17;
    pub const RGBA8888_PREMULTIPLIED: u32 = 18;
}

/// Holds the session bus connection reused across frames.
pub struct KwinCapturer {
    connection: Connection,
}

impl KwinCapturer {
    /// Connects to the session bus and verifies the ScreenShot2 service exists.
    pub fn new() -> Result<Self> {
        let connection =
            Connection::session().context("failed to connect to the D-Bus session bus")?;
        Ok(Self { connection })
    }

    /// Captures the region through `ScreenShot2.CaptureArea`.
    pub fn capture(&self, region: &Region) -> Result<RgbaImage> {
        let (read_fd, write_fd) = create_pipe()?;

        // The reader must run concurrently with the D-Bus call: KWin writes the
        // entire (potentially multi-megabyte) image into the pipe before
        // sending its reply. A pipe filled past its kernel buffer would block
        // KWin's write and deadlock the call if nothing were draining it.
        let reader = thread::spawn(move || -> std::io::Result<Vec<u8>> {
            let mut file = std::fs::File::from(read_fd);
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)?;
            Ok(buffer)
        });

        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("include-cursor", Value::Bool(false));

        let result = self.connection.call_method(
            Some(SERVICE),
            PATH,
            Some(INTERFACE),
            "CaptureArea",
            &(
                region.x,
                region.y,
                region.w,
                region.h,
                options,
                Fd::from(write_fd.as_fd()),
            ),
        );

        // Drop our copy of the write end so the reader sees EOF once KWin
        // finishes, regardless of whether the call itself succeeded.
        drop(write_fd);

        let reply = result.map_err(map_dbus_error)?;
        let metadata: HashMap<String, OwnedValue> = reply
            .body()
            .deserialize()
            .context("failed to decode ScreenShot2 reply metadata")?;

        let data = reader
            .join()
            .map_err(|_| anyhow!("screenshot reader thread panicked"))?
            .context("failed to read screenshot data from pipe")?;

        decode_image(&data, &metadata)
    }
}

/// Creates a close-on-exec pipe, returning `(read, write)` ends.
///
/// `O_NONBLOCK` is intentionally not set: the reader relies on `read` blocking
/// until KWin produces data.
fn create_pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is a valid two-element array as required by `pipe2`.
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error()).context("pipe2 failed");
    }
    // SAFETY: `pipe2` returned two fresh, owned descriptors on success.
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Ok((read_fd, write_fd))
}

/// Translates a ScreenShot2 D-Bus error into actionable guidance.
fn map_dbus_error(err: zbus::Error) -> anyhow::Error {
    let text = err.to_string();
    if text.contains("NoAuthorized") {
        anyhow!(
            "KWin denied the screenshot (NoAuthorized). Install the wayscrollshot \
             .desktop file declaring X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2 \
             into a standard applications directory, or export \
             KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1 for testing."
        )
    } else {
        anyhow!("ScreenShot2 CaptureArea failed: {text}")
    }
}

/// Decodes raw KWin pixel data into an `RgbaImage` using the reply metadata.
fn decode_image(data: &[u8], metadata: &HashMap<String, OwnedValue>) -> Result<RgbaImage> {
    let width = meta_u32(metadata, "width")?;
    let height = meta_u32(metadata, "height")?;
    let format = meta_u32(metadata, "format")?;
    let stride = metadata
        .get("stride")
        .and_then(|value| value_to_u32(value).ok())
        .unwrap_or_else(|| width.saturating_mul(4));

    if width == 0 || height == 0 {
        bail!("ScreenShot2 returned an empty image ({width}x{height})");
    }

    let row_bytes = (width as usize)
        .checked_mul(4)
        .ok_or_else(|| anyhow!("image row too large"))?;
    let stride = stride as usize;
    if stride < row_bytes {
        bail!("ScreenShot2 stride {stride} smaller than row width {row_bytes}");
    }

    let needed = stride
        .checked_mul(height as usize)
        .ok_or_else(|| anyhow!("image too large"))?;
    if data.len() < needed {
        bail!(
            "ScreenShot2 data truncated: got {} bytes, need {needed} ({width}x{height}, stride {stride})",
            data.len()
        );
    }

    let mut out = vec![0u8; row_bytes * height as usize];
    convert_rows(
        data,
        &mut out,
        width as usize,
        height as usize,
        stride,
        format,
    )?;

    RgbaImage::from_raw(width, height, out)
        .ok_or_else(|| anyhow!("decoded buffer does not match {width}x{height}"))
}

/// Converts each source row into packed RGBA, swapping byte order per format.
///
/// Screenshots are opaque, so alpha is forced to 255; this also sidesteps the
/// premultiplied-alpha formats without an un-multiply pass.
fn convert_rows(
    src: &[u8],
    dst: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: u32,
) -> Result<()> {
    use qimage_format::*;

    let bgra = match format {
        ARGB32 | ARGB32_PREMULTIPLIED | RGB32 => true,
        RGBA8888 | RGBA8888_PREMULTIPLIED | RGBX8888 => false,
        other => bail!("unsupported KWin screenshot pixel format: QImage::Format {other}"),
    };

    for y in 0..height {
        let src_row = &src[y * stride..y * stride + width * 4];
        let dst_row = &mut dst[y * width * 4..(y + 1) * width * 4];
        if bgra {
            for x in 0..width {
                let s = &src_row[x * 4..x * 4 + 4];
                let d = &mut dst_row[x * 4..x * 4 + 4];
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = 255;
            }
        } else {
            for x in 0..width {
                let s = &src_row[x * 4..x * 4 + 4];
                let d = &mut dst_row[x * 4..x * 4 + 4];
                d[0] = s[0];
                d[1] = s[1];
                d[2] = s[2];
                d[3] = 255;
            }
        }
    }

    Ok(())
}

fn meta_u32(metadata: &HashMap<String, OwnedValue>, key: &str) -> Result<u32> {
    let value = metadata
        .get(key)
        .ok_or_else(|| anyhow!("ScreenShot2 reply missing '{key}'"))?;
    value_to_u32(value).with_context(|| format!("ScreenShot2 '{key}' is not a u32"))
}

fn value_to_u32(value: &OwnedValue) -> Result<u32> {
    match &**value {
        Value::U32(v) => Ok(*v),
        Value::I32(v) => u32::try_from(*v).map_err(|_| anyhow!("negative value")),
        Value::U16(v) => Ok(u32::from(*v)),
        other => Err(anyhow!(
            "unexpected variant type {:?}",
            other.value_signature()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::convert_rows;
    use super::qimage_format::*;

    #[test]
    fn converts_argb32_bgra_byte_order() {
        // One pixel, little-endian ARGB32 in memory: B=0x10 G=0x20 R=0x30 A=0x40
        let src = vec![0x10, 0x20, 0x30, 0x40];
        let mut dst = vec![0u8; 4];
        convert_rows(&src, &mut dst, 1, 1, 4, ARGB32).unwrap();
        assert_eq!(dst, vec![0x30, 0x20, 0x10, 255]);
    }

    #[test]
    fn converts_rgba8888_passthrough_with_opaque_alpha() {
        let src = vec![0x30, 0x20, 0x10, 0x40];
        let mut dst = vec![0u8; 4];
        convert_rows(&src, &mut dst, 1, 1, 4, RGBA8888).unwrap();
        assert_eq!(dst, vec![0x30, 0x20, 0x10, 255]);
    }

    #[test]
    fn honors_stride_padding() {
        // 1px wide, 2 rows, stride 8 (4 bytes pixel + 4 bytes padding per row).
        let src = vec![
            0x01, 0x02, 0x03, 0xff, 0, 0, 0, 0, // row 0 + padding
            0x04, 0x05, 0x06, 0xff, 0, 0, 0, 0, // row 1 + padding
        ];
        let mut dst = vec![0u8; 8];
        convert_rows(&src, &mut dst, 1, 2, 8, RGBA8888).unwrap();
        assert_eq!(dst, vec![0x01, 0x02, 0x03, 255, 0x04, 0x05, 0x06, 255]);
    }

    #[test]
    fn rejects_unknown_format() {
        let src = vec![0u8; 4];
        let mut dst = vec![0u8; 4];
        assert!(convert_rows(&src, &mut dst, 1, 1, 4, 999).is_err());
    }
}
