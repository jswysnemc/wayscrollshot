//! mark-shot 八带签名拼接。运动掩码会丢掉静止侧栏，固定带检测会去掉页眉页脚。

mod adjacent;
mod algorithm;
mod compare;
mod fixed;
mod pixels;
mod line;
mod long_image;
mod matching;
mod motion;
mod profile;
mod session;
mod types;

#[cfg(test)]
mod tests;

use image::RgbaImage;

use self::pixels::Image;
use self::session::Stitcher as Engine;
use self::types::{Config, Status};
use super::StitchOutcome;
use crate::types::StitchStats;

/// 新拼接算法总开关。关闭后 `-a band` 回退到列采样。
pub const FEATURE_BAND_STITCH_ENABLED: bool = true;

/// 对外的带状拼接会话。
pub struct BandStitcher {
    inner: Engine,
}

impl BandStitcher {
    /// 用 mark-shot 生产阈值创建会话；无参数，返回会话。
    pub fn new() -> Self {
        Self {
            inner: Engine::new(Config::default()).expect("default band config is valid"),
        }
    }

    /// 提交 frame 并返回拼接结果；frame 为新捕获帧。
    pub fn push(&mut self, frame: RgbaImage) -> StitchOutcome {
        let image = Image::from_rgba(&frame);
        match self.inner.push(image) {
            Ok(result) => match result.status {
                Status::FirstFrame => StitchOutcome::FirstFrame,
                Status::Appended => StitchOutcome::Appended {
                    added: result.added.max(0) as u32,
                },
                Status::NoProgress => StitchOutcome::NoProgress,
                Status::NoMatch => StitchOutcome::NoMatch,
            },
            Err(err) => {
                log::error!("【长截图拼接】【带状匹配】提交失败: {err}");
                StitchOutcome::NoMatch
            }
        }
    }

    /// 返回当前长图；无参数，尚无种子帧时返回 None。
    pub fn full_image(&self) -> Option<std::sync::Arc<RgbaImage>> {
        let image = self.inner.full_image().ok().flatten()?;
        Some(std::sync::Arc::new(image.to_rgba()))
    }

    /// 返回累计统计；无参数。
    pub fn stats(&self) -> StitchStats {
        let stats = self.inner.stats();
        StitchStats {
            frame_count: stats.frame_count.max(0) as u32,
            total_height: stats.total_height.max(0) as u32,
            last_append: stats.last_append.max(0) as u32,
        }
    }
}
