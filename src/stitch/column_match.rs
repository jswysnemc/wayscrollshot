use image::RgbaImage;

use super::{predict_offset_iter, ColSamples};

/// 按多组固定列采样截图灰度
///
/// `img` 为原始截图；返回每一行的分组灰度均值
pub(super) fn col_sampling(img: &RgbaImage) -> ColSamples {
    let w = img.width() as usize;
    let h = img.height() as usize;

    if w == 0 || h == 0 {
        return vec![];
    }

    // 1. 生成覆盖左中右区域的采样列组
    let groups = sampling_groups(w);

    // 2. 计算每一行在各采样列组中的灰度均值
    let mut result: Vec<Vec<f32>> = vec![vec![0.0; groups.len()]; h];

    for (group_idx, cols) in groups.iter().enumerate() {
        for y in 0..h {
            let mut sum = 0.0f32;
            let mut count = 0;
            for &x in cols {
                if x < w {
                    let pixel = img.get_pixel(x as u32, y as u32);
                    let gray =
                        0.299 * pixel[0] as f32 + 0.587 * pixel[1] as f32 + 0.114 * pixel[2] as f32;
                    sum += gray;
                    count += 1;
                }
            }
            result[y][group_idx] = if count > 0 { sum / count as f32 } else { 0.0 };
        }
    }

    result
}

/// 生成覆盖截图左中右区域的采样列组
///
/// `width` 为截图宽度；返回三组均匀分布的列索引
fn sampling_groups(width: usize) -> Vec<Vec<usize>> {
    vec![
        linspace(20.min(width - 1), width / 4, 3),
        linspace(width / 2, 5 * width / 8, 3),
        linspace(6 * width / 8, 7 * width / 8, 3),
    ]
}

/// 生成指定区间内均匀分布的列索引
///
/// `start` 和 `end` 为区间边界，`n` 为数量；返回列索引集合
fn linspace(start: usize, end: usize, n: usize) -> Vec<usize> {
    if n == 0 {
        return vec![];
    }
    if n == 1 {
        return vec![start];
    }
    let step = (end.saturating_sub(start)) as f32 / (n - 1) as f32;
    (0..n)
        .map(|i| (start as f32 + i as f32 * step).round() as usize)
        .collect()
}

/// 搜索两组列采样之间的最佳纵向偏移
///
/// `cols1` 和 `cols2` 为相邻截图采样，其他参数控制预测位置和接受范围；返回偏移与差异值
pub(super) fn diff_overlap(
    cols1: &ColSamples,
    cols2: &ColSamples,
    predict: i32,
    approx_diff: f32,
    min_overlap: u32,
) -> (i32, f32) {
    let h1 = cols1.len() as i32;
    let h2 = cols2.len() as i32;

    if h1 == 0 || h2 == 0 {
        return (0, f32::MAX);
    }

    // 1. 根据最小重叠高度确定搜索范围
    let max_offset = (h1 - min_overlap as i32).max(0);
    let mut best = (0i32, f32::MAX);
    let mut approach_count = 0;

    // 2. 从预测位置向两侧搜索差异最小的偏移
    for offset in predict_offset_iter(max_offset, predict) {
        let diff = compute_col_diff(cols1, cols2, offset);

        if diff < best.1 {
            best = (offset, diff);
        }

        if best.1 < approx_diff {
            approach_count += 1;
            if approach_count > 10 {
                return best;
            }
            if diff < approx_diff / 4.0 {
                return best;
            }
        }
    }

    best
}

/// 计算指定纵向偏移下两组列采样的平均绝对差
///
/// `offset` 为待评估偏移；返回平均绝对差，无法比较时返回最大值
fn compute_col_diff(cols1: &ColSamples, cols2: &ColSamples, offset: i32) -> f32 {
    // 1. 校验采样高度和分组数量
    let h1 = cols1.len();
    let h2 = cols2.len();

    if h1 == 0 || h2 == 0 {
        return f32::MAX;
    }

    let num_groups = cols1.first().map(|v| v.len()).unwrap_or(0);
    if num_groups == 0 {
        return f32::MAX;
    }

    let mut sum = 0.0f32;
    let mut count = 0usize;

    // 2. 根据偏移方向选择双方重叠的采样行
    if offset == 0 {
        let len = h1.min(h2);
        for y in 0..len {
            for g in 0..num_groups {
                let diff = (cols1[y][g] - cols2[y][g]).abs();
                sum += diff;
                count += 1;
            }
        }
    } else if offset > 0 {
        let offset_u = offset as usize;
        let len = (h1 - offset_u).min(h2 - offset_u);
        for i in 0..len {
            let y1 = offset_u + i;
            let y2 = i;
            if y1 < h1 && y2 < h2 {
                for g in 0..num_groups {
                    let diff = (cols1[y1][g] - cols2[y2][g]).abs();
                    sum += diff;
                    count += 1;
                }
            }
        }
    } else {
        let offset_u = (-offset) as usize;
        let len = (h1 - offset_u).min(h2 - offset_u);
        for i in 0..len {
            let y1 = i;
            let y2 = offset_u + i;
            if y1 < h1 && y2 < h2 {
                for g in 0..num_groups {
                    let diff = (cols1[y1][g] - cols2[y2][g]).abs();
                    sum += diff;
                    count += 1;
                }
            }
        }
    }

    if count == 0 {
        return f32::MAX;
    }

    // 3. 返回所有有效采样点的平均绝对差
    sum / count as f32
}

/// 按多组固定列采样截图的纵向边缘强度
///
/// `img` 为原始截图；返回每一行的分组边缘均值
pub(super) fn col_sampling_edge(img: &RgbaImage) -> ColSamples {
    let w = img.width() as usize;
    let h = img.height() as usize;

    if w == 0 || h < 2 {
        return vec![];
    }

    // 1. 复用灰度采样使用的左中右列组
    let groups = sampling_groups(w);

    // 2. 计算相邻行在各采样列组中的灰度变化
    let mut result: Vec<Vec<f32>> = vec![vec![0.0; groups.len()]; h];

    for (group_idx, cols) in groups.iter().enumerate() {
        for y in 1..h {
            let mut sum = 0.0f32;
            let mut count = 0;
            for &x in cols {
                if x < w {
                    let curr = img.get_pixel(x as u32, y as u32);
                    let prev = img.get_pixel(x as u32, (y - 1) as u32);

                    let gray_curr =
                        0.299 * curr[0] as f32 + 0.587 * curr[1] as f32 + 0.114 * curr[2] as f32;
                    let gray_prev =
                        0.299 * prev[0] as f32 + 0.587 * prev[1] as f32 + 0.114 * prev[2] as f32;

                    let edge = (gray_curr - gray_prev).abs();
                    sum += edge;
                    count += 1;
                }
            }
            result[y][group_idx] = if count > 0 { sum / count as f32 } else { 0.0 };
        }
        if h > 1 {
            result[0][group_idx] = result[1][group_idx];
        }
    }

    result
}
