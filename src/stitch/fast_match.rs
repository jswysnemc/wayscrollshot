use image::imageops::{self, FilterType};
use image::{GrayImage, RgbaImage};
use imageproc::corners::Corner;

use super::STATIC_DIFF_THRESHOLD;

/// 将截图转换为 FAST 匹配使用的灰度图，并按目标宽度缩放
///
/// `img` 为原始截图，`target_width` 为目标宽度；返回可用于特征提取的灰度图
pub(super) fn prepare_fast_gray(img: &RgbaImage, target_width: u32) -> GrayImage {
    let gray = rgba_to_gray(img);
    let width = gray.width();
    if target_width == 0 || width <= target_width {
        return gray;
    }
    let target_width = target_width.max(1).min(width);
    let height = gray.height();
    imageops::resize(&gray, target_width, height, FilterType::Nearest)
}

/// 按固定步长压缩角点数量
///
/// `corners` 为候选角点，`max_corners` 为数量上限；返回压缩后的角点集合
pub(super) fn downsample_corners(corners: Vec<Corner>, max_corners: usize) -> Vec<Corner> {
    if corners.len() <= max_corners {
        return corners;
    }
    let step = corners.len() / max_corners + 1;
    corners.into_iter().step_by(step).collect()
}

/// 根据相邻截图的像素差过滤静态角点
///
/// `corners` 为候选角点，`gray` 为当前灰度图，`prev_gray` 为上一帧灰度图；返回变化区域中的角点
pub(super) fn filter_corners_by_diff(
    corners: &[Corner],
    gray: &GrayImage,
    prev_gray: &GrayImage,
) -> Vec<Corner> {
    corners
        .iter()
        .filter_map(|corner| {
            if corner.x >= gray.width() || corner.y >= gray.height() {
                return None;
            }
            let curr = gray.get_pixel(corner.x, corner.y)[0];
            let prev = prev_gray.get_pixel(corner.x, corner.y)[0];
            if curr.abs_diff(prev) >= STATIC_DIFF_THRESHOLD {
                Some(*corner)
            } else {
                None
            }
        })
        .collect()
}

/// 将 RGBA 截图转换为灰度图
///
/// `img` 为原始截图；返回保持原始尺寸的灰度图
pub(super) fn rgba_to_gray(img: &RgbaImage) -> GrayImage {
    GrayImage::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y);
        let gray = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) as u8;
        image::Luma([gray])
    })
}
