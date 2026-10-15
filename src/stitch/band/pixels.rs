//! 拼接用的紧凑 RGB 像素缓冲。字节序与 mark-shot 的 `0x00RRGGBB` 一致。

use image::{Rgba, RgbaImage};

pub const MAX_PIXELS: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageError {
    Dimensions,
    Allocation,
    Parameter,
}

impl std::fmt::Display for ImageError {
    /// 将错误 self 格式化到 f，返回格式化结果。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Dimensions => "invalid image dimensions or pixel limit exceeded",
            Self::Allocation => "image allocation failed",
            Self::Parameter => "invalid image parameter",
        })
    }
}

impl std::error::Error for ImageError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    width: i32,
    height: i32,
    pixels: Vec<u32>,
}

impl Image {
    /// 用 width、height、pixels 创建图像；像素必须恰好覆盖整幅图，返回图像或尺寸错误。
    pub fn from_pixels(width: i32, height: i32, pixels: Vec<u32>) -> Result<Self, ImageError> {
        if width <= 0 || height <= 0 || pixels.len() != (width as usize) * (height as usize) {
            return Err(ImageError::Dimensions);
        }
        if pixels.len() > MAX_PIXELS {
            return Err(ImageError::Dimensions);
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// 从 frame 拷贝不透明 RGB；frame 为捕获帧，返回紧凑图像。
    pub fn from_rgba(frame: &RgbaImage) -> Self {
        let width = frame.width() as i32;
        let height = frame.height() as i32;
        let mut pixels = Vec::with_capacity(frame.pixels().len());
        for pixel in frame.pixels() {
            let [r, g, b, _] = pixel.0;
            pixels.push(((r as u32) << 16) | ((g as u32) << 8) | (b as u32));
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    /// 返回宽度；无参数。
    pub fn width(&self) -> i32 {
        self.width
    }

    /// 返回高度；无参数。
    pub fn height(&self) -> i32 {
        self.height
    }

    /// 返回第 y 行像素；y 必须位于图像内。
    pub fn row(&self, y: i32) -> &[u32] {
        assert!(y >= 0 && y < self.height);
        let start = y as usize * self.width as usize;
        &self.pixels[start..start + self.width as usize]
    }

    /// 返回内部像素；无参数。
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// 导出为不透明 RGBA；无参数，返回新图像。
    pub fn to_rgba(&self) -> RgbaImage {
        let mut raw = Vec::with_capacity(self.pixels.len() * 4);
        for px in &self.pixels {
            raw.push(((px >> 16) & 255) as u8);
            raw.push(((px >> 8) & 255) as u8);
            raw.push((px & 255) as u8);
            raw.push(255);
        }
        RgbaImage::from_raw(self.width as u32, self.height as u32, raw).unwrap_or_else(|| {
            RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 255]))
        })
    }
}

/// 分配 len 个像素；失败返回分配错误。
pub fn allocate(len: usize) -> Result<Vec<u32>, ImageError> {
    if len > MAX_PIXELS {
        return Err(ImageError::Dimensions);
    }
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(len)
        .map_err(|_| ImageError::Allocation)?;
    pixels.resize(len, 0);
    Ok(pixels)
}
