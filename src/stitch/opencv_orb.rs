use std::sync::Once;

use image::{GrayImage, RgbaImage};
use opencv::core::{self, Point2f, Rect, Scalar, Vector, CV_8UC1, NORM_HAMMING};
use opencv::features2d;
use opencv::imgproc;
use opencv::opencv_has_inherent_feature_opencl;
use opencv::prelude::*;

use super::fast_match::rgba_to_gray;
use super::{
    OrbEstimate, ORB_BOTTOM_IGNORE_RATIO, ORB_MAX_DX, ORB_MAX_FEATURES, ORB_MAX_GEOMETRY_DRIFT,
    ORB_MIN_IGNORE_PX, ORB_MIN_INLIERS, ORB_MIN_KEYPOINTS, ORB_MIN_MATCHES, ORB_SIDE_IGNORE_RATIO,
    ORB_TOP_IGNORE_RATIO,
};
use crate::opencv_compat::{estimate_affine_partial_2d, RANSAC};

static OPENCV_RUNTIME_INIT: Once = Once::new();

/// 初始化 OpenCV 运行环境并关闭 OpenCL 路径
///
/// 该方法无传参和返回值，在进程内只执行一次
pub(crate) fn init_opencv_runtime() {
    OPENCV_RUNTIME_INIT.call_once(|| {
        std::env::set_var("OPENCV_OPENCL_RUNTIME", "disabled");
        opencv_has_inherent_feature_opencl! {
            if let Err(err) = core::set_use_opencl(false) {
                log::error!("【OpenCV】【关闭 OpenCL】关闭运行时失败: {err}");
            }
        }
    });
}

/// 使用 ORB 特征估算相邻截图的垂直偏移
///
/// `prev` 为上一帧，`frame` 为当前帧，`min_overlap` 为最小重叠高度；返回偏移估算或无匹配结果
pub(super) fn estimate_orb_offset(
    prev: &RgbaImage,
    frame: &RgbaImage,
    min_overlap: u32,
) -> opencv::Result<Option<OrbEstimate>> {
    init_opencv_runtime();

    if prev.width() != frame.width() || prev.height() != frame.height() {
        return Ok(None);
    }
    if prev.width() < 80 || prev.height() < 120 {
        return Ok(None);
    }

    // 1. 转换灰度图并构建特征区域掩码
    let prev_gray = rgba_to_gray(prev);
    let frame_gray = rgba_to_gray(frame);
    let prev_mat = gray_to_mat(&prev_gray)?;
    let frame_mat = gray_to_mat(&frame_gray)?;
    let mask = build_feature_mask(prev.width(), prev.height())?;

    let mut orb = features2d::ORB::create_def()?;
    orb.set_max_features(ORB_MAX_FEATURES)?;

    // 2. 提取相邻截图的 ORB 关键点与描述子
    let mut prev_keypoints = Vector::<core::KeyPoint>::new();
    let mut prev_descriptors = core::Mat::default();
    orb.detect_and_compute_def(&prev_mat, &mask, &mut prev_keypoints, &mut prev_descriptors)?;

    let mut curr_keypoints = Vector::<core::KeyPoint>::new();
    let mut curr_descriptors = core::Mat::default();
    orb.detect_and_compute_def(
        &frame_mat,
        &mask,
        &mut curr_keypoints,
        &mut curr_descriptors,
    )?;

    if prev_keypoints.len() < ORB_MIN_KEYPOINTS
        || curr_keypoints.len() < ORB_MIN_KEYPOINTS
        || prev_descriptors.empty()
        || curr_descriptors.empty()
    {
        return Ok(None);
    }

    let matcher = features2d::BFMatcher::create(NORM_HAMMING, false)?;
    let mut matches = Vector::<Vector<core::DMatch>>::new();
    matcher.knn_train_match_def(&curr_descriptors, &prev_descriptors, &mut matches, 2)?;

    let mut curr_points = Vector::<Point2f>::new();
    let mut prev_points = Vector::<Point2f>::new();
    let mut raw_matches = 0usize;

    // 3. 使用比率测试和垂直滚动约束筛选特征匹配
    for pair in matches.iter() {
        if pair.len() < 2 {
            continue;
        }

        let best = pair.get(0)?;
        let second = pair.get(1)?;

        if best.distance >= second.distance * 0.78 {
            continue;
        }

        let curr_pt = curr_keypoints.get(best.query_idx as usize)?.pt();
        let prev_pt = prev_keypoints.get(best.train_idx as usize)?.pt();
        let dx = (prev_pt.x - curr_pt.x) as f64;
        let dy = (prev_pt.y - curr_pt.y) as f64;

        if dy <= 1.0 || dx.abs() > ORB_MAX_DX * 2.0 {
            continue;
        }

        curr_points.push(curr_pt);
        prev_points.push(prev_pt);
        raw_matches += 1;
    }

    if raw_matches < ORB_MIN_MATCHES {
        return Ok(None);
    }

    let mut inliers = core::Mat::default();
    // 4. 通过部分仿射变换估算垂直偏移
    let affine = estimate_affine_partial_2d(
        &curr_points,
        &prev_points,
        &mut inliers,
        RANSAC,
        3.0,
        2000,
        0.99,
        10,
    )?;

    if affine.empty() {
        return Ok(None);
    }

    let a = *affine.at_2d::<f64>(0, 0)?;
    let b = *affine.at_2d::<f64>(0, 1)?;
    let c = *affine.at_2d::<f64>(1, 0)?;
    let d = *affine.at_2d::<f64>(1, 1)?;
    let tx = *affine.at_2d::<f64>(0, 2)?;
    let ty = *affine.at_2d::<f64>(1, 2)?;

    let scale = ((a * a + c * c).sqrt() + (b * b + d * d).sqrt()) * 0.5;
    let geom_drift = (a - 1.0).abs() + (d - 1.0).abs() + b.abs() + c.abs();

    // 5. 校验水平漂移、缩放和几何形变
    if tx.abs() > ORB_MAX_DX
        || ty <= 1.0
        || ty >= (prev.height() - min_overlap) as f64
        || (scale - 1.0).abs() > ORB_MAX_GEOMETRY_DRIFT
        || geom_drift > ORB_MAX_GEOMETRY_DRIFT
    {
        return Ok(None);
    }

    let mut inlier_count = 0usize;
    for row in 0..inliers.rows() {
        if *inliers.at_2d::<u8>(row, 0)? != 0 {
            inlier_count += 1;
        }
    }

    if inlier_count < ORB_MIN_INLIERS {
        return Ok(None);
    }

    let inlier_ratio = inlier_count as f32 / raw_matches as f32;
    let confidence = (1.0 - inlier_ratio) * 3.5
        + (tx.abs() as f32 / ORB_MAX_DX as f32)
        + (geom_drift as f32 * 6.0);

    Ok(Some(OrbEstimate { dy: ty, confidence }))
}

/// 将灰度图复制到 OpenCV 矩阵
///
/// `gray` 为灰度图；返回 `CV_8UC1` 矩阵
fn gray_to_mat(gray: &GrayImage) -> opencv::Result<core::Mat> {
    let rows = gray.height() as i32;
    let cols = gray.width() as i32;
    let mut mat = core::Mat::new_rows_cols_with_default(rows, cols, CV_8UC1, Scalar::all(0.0))?;

    for y in 0..rows {
        for x in 0..cols {
            *mat.at_2d_mut::<u8>(y, x)? = gray.get_pixel(x as u32, y as u32)[0];
        }
    }

    Ok(mat)
}

/// 构建忽略固定边缘区域的 ORB 特征掩码
///
/// `width` 和 `height` 为截图尺寸；返回单通道掩码矩阵
fn build_feature_mask(width: u32, height: u32) -> opencv::Result<core::Mat> {
    let mut mask = core::Mat::new_rows_cols_with_default(
        height as i32,
        width as i32,
        CV_8UC1,
        Scalar::all(0.0),
    )?;

    let (roi_x, roi_y, roi_w, roi_h) = content_roi(width, height);
    let rect = Rect::new(roi_x as i32, roi_y as i32, roi_w as i32, roi_h as i32);

    imgproc::rectangle(&mut mask, rect, Scalar::all(255.0), -1, imgproc::LINE_8, 0)?;

    Ok(mask)
}

/// 计算排除固定边缘后的内容区域
///
/// `width` 和 `height` 为截图尺寸；返回内容区域的横坐标、纵坐标、宽度和高度
pub(super) fn content_roi(width: u32, height: u32) -> (u32, u32, u32, u32) {
    let side = ((width as f32 * ORB_SIDE_IGNORE_RATIO) as u32).max(ORB_MIN_IGNORE_PX);
    let top = ((height as f32 * ORB_TOP_IGNORE_RATIO) as u32).max(ORB_MIN_IGNORE_PX);
    let bottom = ((height as f32 * ORB_BOTTOM_IGNORE_RATIO) as u32).max(ORB_MIN_IGNORE_PX);
    let x = side.min(width.saturating_sub(1));
    let y = top.min(height.saturating_sub(1));
    let roi_w = width.saturating_sub(x.saturating_mul(2)).max(1);
    let roi_h = height.saturating_sub(y).saturating_sub(bottom).max(1);
    (x, y, roi_w, roi_h)
}
