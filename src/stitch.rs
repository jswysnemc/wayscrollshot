use std::collections::HashMap;
use std::sync::Arc;

use hora::core::ann_index::ANNIndex;
use hora::index::hnsw_idx::HNSWIndex;
use hora::index::hnsw_params::HNSWParams;
use image::imageops::{self, FilterType};
use image::{GenericImage, GrayImage, RgbaImage};
use imageproc::corners::{corners_fast12, corners_fast9};
use rayon::prelude::*;

use crate::cli::Algorithm;
use crate::types::{PreviewImage, StitchStats};

mod column_match;
mod fast_match;
mod opencv_orb;
mod template_match;
#[cfg(test)]
mod tests;

use column_match::{col_sampling, col_sampling_edge, diff_overlap};
use fast_match::{downsample_corners, filter_corners_by_diff, prepare_fast_gray};
use opencv_orb::estimate_orb_offset;
pub(crate) use opencv_orb::init_opencv_runtime;
use template_match::{find_offset_template_content, ncc_score, to_grayscale_vec};

const DESCRIPTOR_PATCH_SIZE: usize = 9;
const DESCRIPTOR_DIM: usize = DESCRIPTOR_PATCH_SIZE & !1;
const CORNER_THRESHOLD: u8 = 64;
const DISTANCE_THRESHOLD: f32 = 0.1;
const MAX_FAST_CORNERS: usize = 1200;
const MIN_FAST_CORNERS: usize = 30;
const STATIC_DIFF_THRESHOLD: u8 = 6;
const DX_TOLERANCE: i32 = 2;
const MIN_OFFSET_FILTER: i32 = 2;
const ORB_MAX_FEATURES: i32 = 1500;
const ORB_MIN_KEYPOINTS: usize = 80;
const ORB_MIN_MATCHES: usize = 24;
const ORB_MIN_INLIERS: usize = 18;
const ORB_MAX_DX: f64 = 12.0;
const ORB_MAX_GEOMETRY_DRIFT: f64 = 0.12;
const ORB_TOP_IGNORE_RATIO: f32 = 0.12;
const ORB_BOTTOM_IGNORE_RATIO: f32 = 0.08;
const ORB_SIDE_IGNORE_RATIO: f32 = 0.04;
const ORB_MIN_IGNORE_PX: u32 = 24;
const TEMPLATE_MIN_HEIGHT: u32 = 48;
const TEMPLATE_FALLBACK_MIN_SCORE: f32 = 0.72;
const TEMPLATE_FALLBACK_MIN_MARGIN: f32 = 0.015;
const TEMPLATE_VERIFY_MAX_DIFF: f32 = 18.0;
const RELAXED_MIN_OVERLAP_FLOOR: u32 = 72;

pub struct MatchConfig {
    pub min_overlap: u32,
    pub accept_diff: f32,
    pub min_append: u32,
    pub approx_diff: f32,
    pub algorithm: Algorithm,
    pub match_width: u32,
}

/// FAST corner index with HNSW
struct FastIndex {
    corners: Vec<(u32, u32)>,
    descriptors: Vec<Vec<f32>>,
    hnsw: HNSWIndex<f32, usize>,
}

impl FastIndex {
    fn new() -> Self {
        let mut params = HNSWParams::<f32>::default();
        params.ef_search = 32;
        params.ef_build = 16;
        let hnsw = HNSWIndex::new(DESCRIPTOR_DIM, &params);
        Self {
            corners: Vec::new(),
            descriptors: Vec::new(),
            hnsw,
        }
    }

    fn build(gray: &GrayImage) -> Self {
        let features = FastFeatures::build(gray, None);
        let mut index = Self::new();

        index.corners = features.corners;
        index.descriptors = features.descriptors;

        for (i, desc) in index.descriptors.iter().enumerate() {
            let _ = index.hnsw.add(desc, i);
        }
        let _ = index.hnsw.build(hora::core::metrics::Metric::Euclidean);

        index
    }
}

struct FastFeatures {
    corners: Vec<(u32, u32)>,
    descriptors: Vec<Vec<f32>>,
}

impl FastFeatures {
    fn build(gray: &GrayImage, prev_gray: Option<&GrayImage>) -> Self {
        let mut index = Self {
            corners: Vec::new(),
            descriptors: Vec::new(),
        };

        // Detect corners using FAST
        let corners_fast12 = corners_fast12(gray, CORNER_THRESHOLD);
        let corners_fast9 = corners_fast9(gray, CORNER_THRESHOLD);
        let mut corners = if corners_fast12.len() > 200 {
            corners_fast12.clone()
        } else {
            corners_fast9.clone()
        };
        let original_corners = corners.clone();

        if let Some(prev) = prev_gray {
            if prev.width() == gray.width() && prev.height() == gray.height() {
                corners = filter_corners_by_diff(&corners, gray, prev);
                if corners.len() < MIN_FAST_CORNERS {
                    corners = original_corners;
                }
            }
        }

        corners = downsample_corners(corners, MAX_FAST_CORNERS);

        // Compute descriptors and build index
        for corner in &corners {
            let desc = compute_descriptor(gray, corner.x, corner.y);
            index.corners.push((corner.x, corner.y));
            index.descriptors.push(desc);
        }

        index
    }
}

/// Compute descriptor for a corner point (row + column features)
fn compute_descriptor(gray: &GrayImage, x: u32, y: u32) -> Vec<f32> {
    let w = gray.width() as i32;
    let h = gray.height() as i32;
    let descriptor_size = DESCRIPTOR_PATCH_SIZE;
    let half_size = descriptor_size as i32 / 2;
    let mut desc = Vec::with_capacity(DESCRIPTOR_DIM);

    // Row features
    for row in 0..(descriptor_size / 2) {
        let yy = y as i32 + (-half_size + row as i32 * 2);
        let mut sum = 0.0;
        let mut count = 0;
        for col in 0..(descriptor_size / 2) {
            let xx = x as i32 + (-half_size + col as i32 * 2);
            if xx >= 0 && xx < w && yy >= 0 && yy < h {
                let pixel = gray.get_pixel(xx as u32, yy as u32)[0] as f32 / 255.0;
                sum += pixel;
                count += 1;
            }
        }
        desc.push(if count > 0 { sum / count as f32 } else { 0.0 });
    }

    // Column features
    for col in 0..(descriptor_size / 2) {
        let xx = x as i32 + (-half_size + col as i32 * 2);
        let mut sum = 0.0;
        let mut count = 0;
        for row in 0..(descriptor_size / 2) {
            let yy = y as i32 + (-half_size + row as i32 * 2);
            if xx >= 0 && xx < w && yy >= 0 && yy < h {
                let pixel = gray.get_pixel(xx as u32, yy as u32)[0] as f32 / 255.0;
                sum += pixel;
                count += 1;
            }
        }
        desc.push(if count > 0 { sum / count as f32 } else { 0.0 });
    }

    desc
}

/// Euclidean distance between two descriptors
fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

pub struct Stitcher {
    full_image: Option<Arc<RgbaImage>>,
    last_frame: Option<RgbaImage>,
    last_cols: Option<ColSamples>,
    last_fast_index: Option<FastIndex>,
    last_fast_gray: Option<GrayImage>,
    last_offset: i32,
    stats: StitchStats,
    config: MatchConfig,
}

pub enum StitchOutcome {
    FirstFrame,
    Appended { added: u32 },
    NoProgress,
    NoMatch,
}

type ColSamples = Vec<Vec<f32>>;

struct OrbEstimate {
    dy: f64,
    confidence: f32,
}

impl Stitcher {
    pub fn new(config: MatchConfig) -> Self {
        Self {
            full_image: None,
            last_frame: None,
            last_cols: None,
            last_fast_index: None,
            last_fast_gray: None,
            last_offset: 0,
            stats: StitchStats {
                frame_count: 0,
                total_height: 0,
                last_append: 0,
            },
            config,
        }
    }

    pub fn push_frame(&mut self, frame: RgbaImage) -> StitchOutcome {
        log::info!(
            "【长截图拼接】【接收帧】截图尺寸为 {}x{}",
            frame.width(),
            frame.height()
        );

        if self.full_image.is_none() {
            let height = frame.height();
            self.stats.frame_count = 1;
            self.stats.total_height = height;
            self.stats.last_append = height;
            self.full_image = Some(Arc::new(frame.clone()));
            self.last_frame = Some(frame.clone());

            match self.config.algorithm {
                Algorithm::Fast => {
                    let gray = prepare_fast_gray(&frame, self.config.match_width);
                    self.last_fast_index = Some(FastIndex::build(&gray));
                    self.last_fast_gray = Some(gray);
                }
                Algorithm::OpenCvOrb => {}
                _ => {
                    self.last_cols = Some(self.compute_cols(&frame));
                }
            }
            return StitchOutcome::FirstFrame;
        }

        let (offset, confidence) = match self.config.algorithm {
            Algorithm::Fast => self.find_offset_fast(&frame),
            Algorithm::Template => self.find_offset_template(&frame),
            Algorithm::OpenCvOrb => self.find_offset_opencv_orb(&frame),
            Algorithm::ColSample | Algorithm::Edge => self.find_offset_colsample(&frame),
        };

        log::info!(
            "【长截图拼接】【偏移估算】偏移为 {}，置信度为 {}",
            offset,
            confidence
        );
        let preserve_anchor = matches!(self.config.algorithm, Algorithm::OpenCvOrb);

        if confidence > self.config.accept_diff {
            if !preserve_anchor {
                self.update_last_frame(frame);
            }
            return StitchOutcome::NoMatch;
        }

        let new_height = if offset > 0 { offset as u32 } else { 0 };

        if new_height < self.config.min_append {
            if !preserve_anchor {
                self.update_last_frame(frame);
                self.last_offset = offset;
            }
            return StitchOutcome::NoProgress;
        }

        // Append new content
        let full = self.full_image.as_ref().expect("full image set");
        let mut combined = RgbaImage::new(full.width(), full.height() + new_height);
        combined
            .copy_from(full.as_ref(), 0, 0)
            .expect("copy full image");

        let overlap = frame.height().saturating_sub(new_height);
        let slice = imageops::crop_imm(&frame, 0, overlap, frame.width(), new_height).to_image();
        combined
            .copy_from(&slice, 0, full.height())
            .expect("copy slice");

        self.full_image = Some(Arc::new(combined));
        self.update_last_frame(frame);
        self.last_offset = offset;
        self.stats.frame_count += 1;
        self.stats.total_height = self.full_image.as_ref().unwrap().height();
        self.stats.last_append = new_height;
        StitchOutcome::Appended { added: new_height }
    }

    fn update_last_frame(&mut self, frame: RgbaImage) {
        match self.config.algorithm {
            Algorithm::Fast => {
                let gray = prepare_fast_gray(&frame, self.config.match_width);
                self.last_fast_index = Some(FastIndex::build(&gray));
                self.last_fast_gray = Some(gray);
            }
            Algorithm::OpenCvOrb => {}
            _ => {
                self.last_cols = Some(self.compute_cols(&frame));
            }
        }
        self.last_frame = Some(frame);
    }

    fn compute_cols(&self, frame: &RgbaImage) -> ColSamples {
        match self.config.algorithm {
            Algorithm::Edge => col_sampling_edge(frame),
            _ => col_sampling(frame),
        }
    }

    /// FAST corner + HNSW matching (from snow-shot)
    fn find_offset_fast(&self, frame: &RgbaImage) -> (i32, f32) {
        let prev_index = match &self.last_fast_index {
            Some(idx) => idx,
            None => return (0, f32::MAX),
        };

        if prev_index.corners.is_empty() {
            return (0, f32::MAX);
        }

        let gray = prepare_fast_gray(frame, self.config.match_width);
        let curr_features = FastFeatures::build(&gray, self.last_fast_gray.as_ref());

        if curr_features.corners.is_empty() {
            return (0, f32::MAX);
        }

        // Match features using HNSW
        let offsets: Vec<i32> = curr_features
            .descriptors
            .par_iter()
            .enumerate()
            .filter_map(|(i, desc)| {
                let search_result = prev_index.hnsw.search(desc, 1);
                if search_result.is_empty() {
                    return None;
                }
                let idx = search_result[0];
                let dist = euclidean_distance(&prev_index.descriptors[idx], desc);

                if dist > DISTANCE_THRESHOLD {
                    return None;
                }

                // Calculate Y offset (vertical scroll)
                let (prev_x, prev_y) = prev_index.corners[idx];
                let (curr_x, curr_y) = curr_features.corners[i];
                if (curr_x as i32 - prev_x as i32).abs() > DX_TOLERANCE {
                    return None;
                }
                let dy = curr_y as i32 - prev_y as i32;

                // For vertical scrolling, we expect negative dy (content moves up)
                let offset = -dy;
                if offset < MIN_OFFSET_FILTER {
                    return None;
                }
                Some(offset)
            })
            .collect();

        if offsets.is_empty() {
            log::info!("【长截图拼接】【FAST 匹配】未找到有效偏移");
            return (0, f32::MAX);
        }

        // Frequency voting: find most common offset
        let mut counts: HashMap<i32, i32> = HashMap::new();
        for &offset in &offsets {
            *counts.entry(offset).or_insert(0) += 1;
        }

        let mut sorted: Vec<_> = counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));

        let (best_offset, best_count) = sorted[0];
        let second_count = sorted.get(1).map(|(_, c)| *c).unwrap_or(0);

        log::info!(
            "【长截图拼接】【FAST 匹配】角点数为 {}，偏移样本数为 {}，最佳偏移为 {}，最佳票数为 {}，次高票数为 {}",
            curr_features.corners.len(),
            offsets.len(),
            best_offset,
            best_count,
            second_count
        );

        // Confidence checks
        let min_matches = (curr_features.corners.len() as i32 / 10).max(3);
        if best_count < min_matches {
            log::info!(
                "【长截图拼接】【FAST 匹配】最佳票数 {} 小于最低匹配数 {}",
                best_count,
                min_matches
            );
            return (0, f32::MAX);
        }

        // Avoid ambiguity
        if best_count < second_count * 2 {
            log::info!("【长截图拼接】【FAST 匹配】匹配结果存在歧义");
            return (0, f32::MAX);
        }

        // Convert count to confidence (lower is better for our interface)
        let confidence = 1.0 - (best_count as f32 / offsets.len() as f32);

        (best_offset, confidence * 10.0)
    }

    fn find_offset_colsample(&self, frame: &RgbaImage) -> (i32, f32) {
        let cols = self.compute_cols(frame);
        let last_cols = match &self.last_cols {
            Some(c) => c,
            None => return (0, f32::MAX),
        };

        diff_overlap(
            last_cols,
            &cols,
            self.last_offset,
            self.config.approx_diff,
            self.config.min_overlap,
        )
    }

    fn find_offset_template(&self, frame: &RgbaImage) -> (i32, f32) {
        let prev = match &self.last_frame {
            Some(f) => f,
            None => return (0, f32::MAX),
        };

        let h = prev.height() as i32;
        let w = prev.width() as i32;

        if h < 100 || w < 50 {
            return (0, f32::MAX);
        }

        let skip_top = (h as f32 * 0.05) as u32;
        let template_height = (h as f32 * 0.20) as u32;
        let template = imageops::crop_imm(frame, 0, skip_top, w as u32, template_height).to_image();
        let template_gray = to_grayscale_vec(&template);

        let prev_gray = to_grayscale_vec(prev);

        let search_start = skip_top as i32;
        let search_end = h - template_height as i32;

        if search_end <= search_start {
            return (0, f32::MAX);
        }

        let mut best_offset = 0i32;
        let mut best_score = f32::MIN;

        let predict = self.last_offset.clamp(0, search_end - search_start);
        let offsets = predict_offset_iter(search_end - search_start, predict);

        for offset in offsets {
            let search_y = search_start + offset;
            if search_y < 0 || search_y + template_height as i32 > h {
                continue;
            }

            let score = ncc_score(&prev_gray, &template_gray, search_y as u32, w as u32);

            if score > best_score {
                best_score = score;
                best_offset = offset;
            }

            if best_score > 0.95 {
                break;
            }
        }

        let diff = 1.0 - best_score.max(0.0);
        (best_offset, diff * 10.0)
    }

    fn find_offset_opencv_orb(&self, frame: &RgbaImage) -> (i32, f32) {
        let prev = match &self.last_frame {
            Some(f) => f,
            None => return (0, f32::MAX),
        };

        match estimate_orb_offset(prev, frame, self.config.min_overlap) {
            Ok(Some(estimate)) => (estimate.dy.round() as i32, estimate.confidence),
            Ok(None) => {
                if let Some((offset, confidence)) = self.find_offset_opencv_relaxed(prev, frame) {
                    return (offset, confidence);
                }
                self.find_offset_template_fallback(prev, frame)
            }
            Err(err) => {
                log::error!("【长截图拼接】【OpenCV ORB 匹配】匹配失败: {err}");
                if let Some((offset, confidence)) = self.find_offset_opencv_relaxed(prev, frame) {
                    return (offset, confidence);
                }
                self.find_offset_template_fallback(prev, frame)
            }
        }
    }

    fn find_offset_opencv_relaxed(
        &self,
        prev: &RgbaImage,
        frame: &RgbaImage,
    ) -> Option<(i32, f32)> {
        if self.config.min_overlap <= RELAXED_MIN_OVERLAP_FLOOR {
            return None;
        }

        let relaxed_overlap = self
            .config
            .min_overlap
            .saturating_sub(40)
            .max(RELAXED_MIN_OVERLAP_FLOOR);

        match estimate_orb_offset(prev, frame, relaxed_overlap) {
            Ok(Some(estimate)) => {
                let confidence = estimate.confidence + 0.45;
                Some((estimate.dy.round() as i32, confidence))
            }
            Ok(None) => None,
            Err(err) => {
                log::error!("【长截图拼接】【OpenCV ORB 宽松匹配】匹配失败: {err}");
                None
            }
        }
    }

    fn find_offset_template_fallback(&self, prev: &RgbaImage, frame: &RgbaImage) -> (i32, f32) {
        let Some((offset, confidence)) =
            find_offset_template_content(prev, frame, self.last_offset, self.config.min_overlap)
        else {
            return self.find_offset_template(frame);
        };
        (offset, confidence)
    }

    pub fn full_image(&self) -> Option<Arc<RgbaImage>> {
        self.full_image.clone()
    }

    pub fn stats(&self) -> StitchStats {
        self.stats.clone()
    }
}

pub fn build_preview(image: &RgbaImage, fixed_width: u32) -> PreviewImage {
    let width = image.width();
    let height = image.height();
    let scale = (fixed_width as f32) / (width as f32).max(1.0);
    let target_width = fixed_width.max(1);
    let target_height = ((height as f32) * scale).round().max(1.0) as u32;
    let resized = imageops::resize(image, target_width, target_height, FilterType::Triangle);
    PreviewImage {
        width: resized.width(),
        height: resized.height(),
        pixels: resized.into_raw(),
    }
}

fn predict_offset_iter(max: i32, predict: i32) -> Vec<i32> {
    let p = predict.clamp(0, max);
    let mut result = vec![p];

    for delta in 1..=max {
        if p + delta <= max {
            result.push(p + delta);
        }
        if p - delta >= 0 {
            result.push(p - delta);
        }
    }

    result
}
