use image::RgbaImage;

use super::opencv_orb::content_roi;
use super::{
    predict_offset_iter, TEMPLATE_FALLBACK_MIN_MARGIN, TEMPLATE_FALLBACK_MIN_SCORE,
    TEMPLATE_MIN_HEIGHT, TEMPLATE_VERIFY_MAX_DIFF,
};

/// 将截图转换为灰度浮点数组
///
/// `img` 为原始截图；返回按行排列的灰度值
pub(super) fn to_grayscale_vec(img: &RgbaImage) -> Vec<f32> {
    img.pixels()
        .map(|p| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32)
        .collect()
}

/// 在内容区域内执行模板匹配回退
///
/// `prev` 和 `frame` 为相邻截图，`predict` 为预测偏移，`min_overlap` 为最小重叠高度；返回偏移和置信度
pub(super) fn find_offset_template_content(
    prev: &RgbaImage,
    frame: &RgbaImage,
    predict: i32,
    min_overlap: u32,
) -> Option<(i32, f32)> {
    if prev.width() != frame.width() || prev.height() != frame.height() {
        return None;
    }

    // 1. 确定排除固定边缘后的模板与搜索区域
    let width = prev.width();
    let height = prev.height();
    let (roi_x, roi_y, roi_w, roi_h) = content_roi(width, height);
    if roi_h < TEMPLATE_MIN_HEIGHT * 2 || roi_w < 40 {
        return None;
    }

    let template_h = (roi_h / 3).max(TEMPLATE_MIN_HEIGHT).min(roi_h - 1);
    let search_start = roi_y as i32;
    let search_end = (roi_y + roi_h - template_h) as i32;
    if search_end <= search_start {
        return None;
    }

    let prev_gray = to_grayscale_vec(prev);
    let frame_gray = to_grayscale_vec(frame);
    let frame_template_y = roi_y;

    let mut best_offset = 0i32;
    let mut best_score = f32::MIN;
    let mut second_score = f32::MIN;

    let max_offset = (height as i32 - min_overlap as i32).max(0);
    let predict = predict.clamp(0, max_offset.min(search_end - search_start));
    // 2. 从预测位置向两侧搜索归一化互相关最高的偏移
    for offset in predict_offset_iter(search_end - search_start, predict) {
        let search_y = search_start + offset;
        if search_y < 0 || search_y + template_h as i32 > height as i32 {
            continue;
        }

        let score = ncc_score_region(
            &prev_gray,
            &frame_gray,
            width,
            roi_x,
            roi_w,
            search_y as u32,
            frame_template_y,
            template_h,
        );

        if score > best_score {
            second_score = best_score;
            best_score = score;
            best_offset = offset;
        } else if score > second_score {
            second_score = score;
        }
    }

    if best_score < TEMPLATE_FALLBACK_MIN_SCORE {
        return None;
    }

    if second_score.is_finite() && best_score - second_score < TEMPLATE_FALLBACK_MIN_MARGIN {
        return None;
    }

    // 3. 使用重叠区域平均绝对差排除误匹配
    let verification = overlap_mean_abs_diff(
        &prev_gray,
        &frame_gray,
        width,
        roi_x,
        roi_w,
        best_offset as u32,
        height.saturating_sub(best_offset as u32),
    );

    if !verification.is_finite() || verification > TEMPLATE_VERIFY_MAX_DIFF {
        return None;
    }

    let confidence = (1.0 - best_score.max(0.0)) * 8.0 + verification / 10.0;
    Some((best_offset, confidence))
}

/// 计算整行模板与指定纵向位置的归一化互相关得分
///
/// 两个灰度数组按 `width` 排列，`y_offset` 为图像起始行；返回互相关得分
pub(super) fn ncc_score(
    image_gray: &[f32],
    template_gray: &[f32],
    y_offset: u32,
    width: u32,
) -> f32 {
    // 1. 计算模板区域的均值和标准差
    let tmpl_len = template_gray.len();
    if tmpl_len == 0 {
        return f32::MIN;
    }

    let tmpl_mean: f32 = template_gray.iter().sum::<f32>() / tmpl_len as f32;
    let tmpl_var: f32 = template_gray
        .iter()
        .map(|&v| (v - tmpl_mean).powi(2))
        .sum::<f32>()
        / tmpl_len as f32;
    let tmpl_std = tmpl_var.sqrt();

    if tmpl_std < 1.0 {
        return f32::MIN;
    }

    let start_idx = (y_offset as usize) * (width as usize);
    let end_idx = start_idx + tmpl_len;

    if end_idx > image_gray.len() {
        return f32::MIN;
    }

    let mut img_sum = 0.0f32;
    let mut sum_img_sq = 0.0f32;

    // 2. 计算待比较图像区域的均值和标准差
    for i in 0..tmpl_len {
        let img_val = image_gray[start_idx + i];
        img_sum += img_val;
        sum_img_sq += img_val * img_val;
    }

    let img_mean = img_sum / tmpl_len as f32;
    let img_var = sum_img_sq / tmpl_len as f32 - img_mean * img_mean;
    let img_std = img_var.max(0.0).sqrt();

    if img_std < 1.0 {
        return f32::MIN;
    }

    let mut ncc = 0.0f32;
    // 3. 计算双方归一化互相关得分
    for (i, &tmpl_val) in template_gray.iter().enumerate() {
        let img_val = image_gray[start_idx + i];
        ncc += (tmpl_val - tmpl_mean) * (img_val - img_mean);
    }

    ncc / (tmpl_len as f32 * tmpl_std * img_std)
}

/// 计算两个内容子区域的归一化互相关得分
///
/// 参数描述图像宽度、内容横向范围和两个纵向起点；返回互相关得分
fn ncc_score_region(
    image_gray: &[f32],
    template_gray: &[f32],
    width: u32,
    roi_x: u32,
    roi_w: u32,
    image_y: u32,
    template_y: u32,
    template_h: u32,
) -> f32 {
    if roi_w == 0 || template_h == 0 || width == 0 {
        return f32::MIN;
    }

    // 1. 计算模板区域与图像区域的平均灰度
    let mut tmpl_sum = 0.0f32;
    let mut img_sum = 0.0f32;
    let mut count = 0usize;

    for row in 0..template_h {
        let tmpl_base = ((template_y + row) * width + roi_x) as usize;
        let img_base = ((image_y + row) * width + roi_x) as usize;
        for col in 0..roi_w as usize {
            tmpl_sum += template_gray[tmpl_base + col];
            img_sum += image_gray[img_base + col];
            count += 1;
        }
    }

    if count == 0 {
        return f32::MIN;
    }

    let tmpl_mean = tmpl_sum / count as f32;
    let img_mean = img_sum / count as f32;
    let mut numerator = 0.0f32;
    let mut tmpl_var = 0.0f32;
    let mut img_var = 0.0f32;

    // 2. 计算协方差与双方差
    for row in 0..template_h {
        let tmpl_base = ((template_y + row) * width + roi_x) as usize;
        let img_base = ((image_y + row) * width + roi_x) as usize;
        for col in 0..roi_w as usize {
            let tmpl = template_gray[tmpl_base + col] - tmpl_mean;
            let img = image_gray[img_base + col] - img_mean;
            numerator += tmpl * img;
            tmpl_var += tmpl * tmpl;
            img_var += img * img;
        }
    }

    if tmpl_var <= 1.0 || img_var <= 1.0 {
        return f32::MIN;
    }

    numerator / (tmpl_var.sqrt() * img_var.sqrt())
}

/// 计算相邻截图重叠区域的平均绝对灰度差
///
/// 参数描述灰度数组、内容横向范围、偏移与重叠高度；返回平均绝对差值
fn overlap_mean_abs_diff(
    prev_gray: &[f32],
    frame_gray: &[f32],
    width: u32,
    roi_x: u32,
    roi_w: u32,
    offset: u32,
    overlap_h: u32,
) -> f32 {
    if roi_w == 0 || overlap_h == 0 {
        return f32::MAX;
    }

    // 1. 从重叠区域底部选取固定高度的验证样本
    let sample_h = overlap_h.min(160);
    let start_prev_y = offset + overlap_h.saturating_sub(sample_h);
    let start_frame_y = overlap_h.saturating_sub(sample_h);
    let mut sum = 0.0f32;
    let mut count = 0usize;

    // 2. 累加双方内容区域的绝对灰度差
    for row in 0..sample_h {
        let prev_base = ((start_prev_y + row) * width + roi_x) as usize;
        let frame_base = ((start_frame_y + row) * width + roi_x) as usize;
        for col in 0..roi_w as usize {
            sum += (prev_gray[prev_base + col] - frame_gray[frame_base + col]).abs();
            count += 1;
        }
    }

    if count == 0 {
        return f32::MAX;
    }

    // 3. 返回验证样本的平均绝对差
    sum / count as f32
}
