use super::compare::{overlap_diff, Window};
use super::fixed::FixedRegions;
use super::pixels::{Image, ImageError};
use super::long_image::LongImage;
use super::motion::MotionMask;
use super::profile::Signature;
use super::types::{Config, Edge, Stats, Status, StitchResult};

pub struct Stitcher {
    pub(crate) config: Config,
    pub(crate) long: LongImage,
    pub(crate) last_frame: Option<Image>,
    pub(crate) last_cols: Signature,
    pub(crate) fixed: FixedRegions,
    pub(crate) motion: MotionMask,
    pub(crate) last_offset: i32,
    pub(crate) best_bottom_trim: i32,
    pub(crate) best_top_trim: i32,
    pub(crate) pending_edge: Edge,
    pub(crate) growth_edge: Edge,
    pub(crate) anchor: i32,
    pub(crate) stats: Stats,
}

impl Stitcher {
    /// 用 config 创建独立拼接会话，返回会话或非法配置错误。
    pub fn new(config: Config) -> Result<Self, ImageError> {
        if !config.valid() {
            return Err(ImageError::Parameter);
        }
        Ok(Self {
            config,
            long: LongImage::default(),
            last_frame: None,
            last_cols: Vec::new(),
            fixed: FixedRegions::default(),
            motion: MotionMask::default(),
            last_offset: 0,
            best_bottom_trim: 0,
            best_top_trim: 0,
            pending_edge: Edge::None,
            growth_edge: Edge::None,
            anchor: 0,
            stats: Stats::default(),
        })
    }

    /// 返回会话当前统计；无参数。
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// 返回最终图像；无参数，未初始化返回 None，失败返回内存错误。
    pub fn full_image(&self) -> Result<Option<Image>, ImageError> {
        if self.long.height() == 0 {
            return Ok(None);
        }
        Ok(Some(self.long.image()?))
    }

    /// 用 frame、cols 设置第一帧，返回种子帧结果或内存错误。
    pub(crate) fn initialize(
        &mut self,
        frame: Image,
        cols: Signature,
    ) -> Result<StitchResult, ImageError> {
        self.long.reset(&frame, cols.clone())?;
        self.fixed.reset();
        self.motion = MotionMask::default();
        self.anchor = 0;
        self.best_bottom_trim = 0;
        self.best_top_trim = 0;
        self.pending_edge = Edge::None;
        self.growth_edge = Edge::None;
        let height = frame.height();
        self.stats = Stats {
            frame_count: 1,
            total_height: height,
            last_append: height,
        };
        self.remember(frame, cols);
        Ok(StitchResult {
            status: Status::FirstFrame,
            added: height,
            edge: Edge::None,
            position: 0,
            frame_length: height,
        })
    }

    /// 保存 frame、cols 作为下一帧匹配基准；无返回值。
    pub(crate) fn remember(&mut self, frame: Image, cols: Signature) {
        self.last_frame = Some(frame);
        self.last_cols = cols;
    }

    /// 根据当前固定带与配置构造比较窗口；无参数，返回窗口。
    pub(crate) fn window(&self) -> Window {
        Window {
            minimum: self.config.min_overlap,
            top: self.fixed.top(),
            bottom: self.fixed.bottom(),
        }
    }

    /// 复核 cols 在 pos 处与长图的重叠，excluded 为旧固定带，返回误差及长度。
    pub(crate) fn known_diff(
        &self,
        cols: &Signature,
        pos: i32,
        excluded: (i32, i32),
    ) -> (f32, i32) {
        overlap_diff(
            &self.long.cols,
            cols,
            pos,
            self.window(),
            excluded,
            &self.motion.weights,
        )
    }

    /// 判断 pos 对 height 高帧是否在锚点可达范围内；返回可达性。
    pub(crate) fn reachable(&self, pos: i32, height: i32) -> bool {
        (pos - self.anchor).abs() <= height - self.config.min_overlap
    }
}
