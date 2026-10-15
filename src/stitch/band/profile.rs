use super::pixels::{Image, ImageError, MAX_PIXELS};
use super::types::NO_MATCH;

pub const BAND_COUNT: usize = 8;
pub type Signature = Vec<[f32; BAND_COUNT]>;
pub type Weights = [f32; BAND_COUNT];
pub const ALL_BANDS: Weights = [1.0; BAND_COUNT];
pub const ROW_MAX_DIFF: f32 = 2.0;

/// 将图像 width 上第 band 个采样带转换为包含两端的列范围；返回起止列。
pub fn band_columns(width: i32, band: usize) -> (i32, i32) {
    if width <= 0 {
        return (0, 0);
    }
    let span = (0.96_f32 - 0.04_f32) / BAND_COUNT as f32;
    let start = 0.04 + span * band as f32;
    let first = (width as f32 * start).round() as i32;
    let last = (width as f32 * (start + span * 0.95)).round() as i32;
    (
        first.clamp(0, width - 1),
        last.clamp(first.clamp(0, width - 1), width - 1),
    )
}

/// 【长截图】【行签名】采样 frame 的八个窄带，返回行亮度签名或内存预算错误。
pub fn compute(frame: &Image) -> Result<Signature, ImageError> {
    let rows = frame.height() as usize;
    if (frame.width() as usize + BAND_COUNT)
        .checked_mul(rows)
        .map(|count| count > MAX_PIXELS)
        .unwrap_or(true)
    {
        return Err(ImageError::Dimensions);
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(rows)
        .map_err(|_| ImageError::Allocation)?;
    result.resize(rows, [0.0; BAND_COUNT]);
    for band in 0..BAND_COUNT {
        let (start, end) = band_columns(frame.width(), band);
        let count = (end - start + 1).clamp(1, 9);
        let step = if count > 1 {
            (end - start) as f32 / (count - 1) as f32
        } else {
            0.0
        };
        for (y, row) in result.iter_mut().enumerate() {
            let mut sum = 0.0;
            for i in 0..count {
                let x =
                    ((start as f32 + i as f32 * step).round() as i32).clamp(0, frame.width() - 1);
                let pixel = frame.row(y as i32)[x as usize];
                sum += 0.299 * ((pixel >> 16) & 255) as f32
                    + 0.587 * ((pixel >> 8) & 255) as f32
                    + 0.114 * (pixel & 255) as f32;
            }
            row[band] = sum / count as f32;
        }
    }
    Ok(result)
}

/// 按 height、ratio 计算软裁剪边缘高度；返回不超过四分之一帧高的行数。
fn scaled_ignore(height: i32, ratio: f32) -> i32 {
    if height < 80 {
        0
    } else {
        (height / 4).min(16.max((height as f32 * ratio) as i32))
    }
}

/// 返回 height 对应的顶部软裁剪行数。
pub fn top_ignore(height: i32) -> i32 {
    scaled_ignore(height, 0.10)
}

/// 返回 height 对应的底部软裁剪行数。
pub fn bottom_ignore(height: i32) -> i32 {
    scaled_ignore(height, 0.08)
}

/// 返回 width 对应的两侧忽略宽度，避免边框与滚动条影响像素比较。
pub fn side_ignore(width: i32) -> i32 {
    if width <= 0 {
        0
    } else {
        50.max(width / 20)
            .min(width / 3)
            .min(((width - 1) / 2).max(0))
    }
}

/// 比较行 a、b 在 start 起 width 列的 RGB 平均绝对差；非法范围返回无匹配置信度。
pub fn row_diff(a: &[u32], b: &[u32], start: i32, width: i32) -> f32 {
    if start < 0
        || width <= 0
        || (i64::from(start) + i64::from(width)) as usize > a.len().min(b.len())
    {
        return NO_MATCH;
    }
    let mut sum = 0.0;
    let mut count = 0;
    for x in (start as usize..(start + width) as usize).step_by((width / 256).max(1) as usize) {
        for shift in [16, 8, 0] {
            sum += (((a[x] >> shift) & 255) as i32 - ((b[x] >> shift) & 255) as i32).abs() as f32;
            count += 1;
        }
    }
    if count > 0 {
        sum / count as f32
    } else {
        NO_MATCH
    }
}
