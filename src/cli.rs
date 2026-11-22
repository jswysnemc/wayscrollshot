use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, ValueEnum};

use crate::constants::PREVIEW_MAX_WIDTH;

/// 使用 `--no-preview` 且未显式指定空闲时间时的默认自动完成等待时长
pub const DEFAULT_NO_PREVIEW_IDLE_DURATION: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum Algorithm {
    /// Column sampling (fast, good for most cases)
    ColSample,
    /// Template matching (slower, more accurate)
    Template,
    /// Edge detection (for transparent backgrounds)
    Edge,
    /// FAST corner + HNSW index (high accuracy, from snow-shot)
    Fast,
    /// OpenCV ORB + brute-force matching + affine RANSAC
    #[value(name = "opencv-orb")]
    OpenCvOrb,
    /// Eight-band signature match. Ignores static sidebars and fixed header/footer bars.
    #[default]
    Band,
}

/// Screen capture backend.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum Backend {
    /// Detect from the desktop session (KDE uses kwin, otherwise grim)
    #[default]
    Auto,
    /// grim / wlr-screencopy (Sway, Hyprland, niri, river, ...)
    Grim,
    /// org.kde.KWin.ScreenShot2 D-Bus interface (KDE Plasma)
    Kwin,
    /// GNOME Shell scroll helper (requires the bundled extension)
    Gnome,
}

#[derive(Parser, Debug, Clone)]
#[command(name = "wayscrollshot")]
#[command(about = "A scrolling screenshot tool for Wayland", long_about = None)]
pub struct Args {
    /// Output file path (default: ~/Pictures/wayscrollshot/wayscrollshot-<timestamp>.png).
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Preview width in pixels
    #[arg(short = 'w', long, default_value_t = PREVIEW_MAX_WIDTH)]
    pub preview_width: u32,

    /// Copy to clipboard instead of saving to file
    #[arg(short, long)]
    pub clipboard: bool,

    /// Disable preview window (auto-saves after idle unless --auto-save-after-idle overrides the duration)
    #[arg(long)]
    pub no_preview: bool,

    /// Automatically save (or copy with -c) and exit after this idle duration without scroll updates (e.g. 2s, 1500ms, 2.5)
    #[arg(long, value_name = "DURATION", value_parser = parse_idle_duration)]
    pub auto_save_after_idle: Option<Duration>,

    /// Disable region border overlay
    #[arg(long)]
    pub no_border: bool,

    /// Stitching algorithm to use
    #[arg(short, long, value_enum, default_value_t = Algorithm::OpenCvOrb)]
    pub algorithm: Algorithm,

    /// Screen capture backend (auto detects KDE vs wlroots)
    #[arg(long, value_enum, default_value_t = Backend::Auto)]
    pub backend: Backend,

    /// Existing slurp geometry to use instead of selecting a region. Use '-' to read from stdin.
    #[arg(value_name = "REGION", num_args = 0..=2, allow_hyphen_values = true)]
    pub region: Vec<String>,
}

impl Args {
    pub fn parse_args() -> Self {
        Args::parse()
    }

    pub fn slurp_output(&self) -> Option<String> {
        if self.region.is_empty() {
            None
        } else {
            Some(self.region.join(" "))
        }
    }

    /// 计算当前会话生效的空闲自动完成时长
    ///
    /// 该方法无传参；返回生效的空闲等待时长，`None` 表示不启用自动完成
    pub fn effective_auto_save_idle(&self) -> Option<Duration> {
        if let Some(duration) = self.auto_save_after_idle {
            return Some(duration);
        }
        if self.no_preview {
            return Some(DEFAULT_NO_PREVIEW_IDLE_DURATION);
        }
        None
    }
}

/// 解析空闲自动完成时长参数，支持纯秒数以及 `ms`、`s`、`m` 后缀
///
/// `raw` 为命令行传入的时长字符串；成功返回正数 `Duration`，失败返回错误说明
pub fn parse_idle_duration(raw: &str) -> Result<Duration, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("duration cannot be empty".to_string());
    }

    // 1. 提取数值部分与单位换算倍率
    let (num_str, multiplier_secs) = if let Some(v) = trimmed.strip_suffix("ms") {
        (v.trim(), 0.001_f64)
    } else if let Some(v) = trimmed.strip_suffix('s') {
        (v.trim(), 1.0_f64)
    } else if let Some(v) = trimmed.strip_suffix('m') {
        (v.trim(), 60.0_f64)
    } else {
        (trimmed, 1.0_f64)
    };

    // 2. 校验数值有效性与正数约束
    let value: f64 = num_str
        .parse()
        .map_err(|_| format!("invalid duration '{raw}' (expected e.g. 2s, 1500ms, 2.5)"))?;
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("duration must be greater than 0, got '{raw}'"));
    }

    let secs = value * multiplier_secs;
    if !secs.is_finite() || secs <= 0.0 {
        return Err(format!("duration out of range: '{raw}'"));
    }

    Ok(Duration::from_secs_f64(secs))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;

    #[test]
    fn parses_quoted_slurp_region() {
        let args = Args::try_parse_from(["wayscrollshot", "10,20 300x400"]).unwrap();

        assert_eq!(args.slurp_output().as_deref(), Some("10,20 300x400"));
    }

    #[test]
    fn parses_unquoted_slurp_region_parts() {
        let args = Args::try_parse_from(["wayscrollshot", "10,20", "300x400"]).unwrap();

        assert_eq!(args.slurp_output().as_deref(), Some("10,20 300x400"));
    }

    #[test]
    fn parses_stdin_region_marker() {
        let args = Args::try_parse_from(["wayscrollshot", "-"]).unwrap();

        assert_eq!(args.slurp_output().as_deref(), Some("-"));
    }

    #[test]
    fn parses_negative_slurp_coordinates() {
        let args = Args::try_parse_from(["wayscrollshot", "-1920,-10", "800x600"]).unwrap();

        assert_eq!(args.slurp_output().as_deref(), Some("-1920,-10 800x600"));
    }

    #[test]
    fn keeps_existing_flags_with_region() {
        let args = Args::try_parse_from(["wayscrollshot", "-c", "10,20", "300x400"]).unwrap();

        assert!(args.clipboard);
        assert_eq!(args.slurp_output().as_deref(), Some("10,20 300x400"));
    }

    #[test]
    fn no_preview_enables_default_idle_auto_save() {
        let args = Args::try_parse_from(["wayscrollshot", "--no-preview"]).unwrap();

        assert_eq!(
            args.effective_auto_save_idle(),
            Some(super::DEFAULT_NO_PREVIEW_IDLE_DURATION)
        );
    }

    #[test]
    fn interactive_mode_disables_idle_auto_save_by_default() {
        let args = Args::try_parse_from(["wayscrollshot"]).unwrap();

        assert_eq!(args.effective_auto_save_idle(), None);
    }

    #[test]
    fn parses_auto_save_after_idle_durations() {
        use std::time::Duration;

        let args_sec =
            Args::try_parse_from(["wayscrollshot", "--auto-save-after-idle", "3s"]).unwrap();
        assert_eq!(args_sec.effective_auto_save_idle(), Some(Duration::from_secs(3)));

        let args_ms =
            Args::try_parse_from(["wayscrollshot", "--auto-save-after-idle", "1500ms"]).unwrap();
        assert_eq!(
            args_ms.effective_auto_save_idle(),
            Some(Duration::from_millis(1500))
        );

        let args_num =
            Args::try_parse_from(["wayscrollshot", "--auto-save-after-idle", "2.5"]).unwrap();
        assert_eq!(
            args_num.effective_auto_save_idle(),
            Some(Duration::from_millis(2500))
        );

        let args_min =
            Args::try_parse_from(["wayscrollshot", "--auto-save-after-idle", "1m"]).unwrap();
        assert_eq!(
            args_min.effective_auto_save_idle(),
            Some(Duration::from_secs(60))
        );
    }

    #[test]
    fn rejects_invalid_auto_save_after_idle() {
        assert!(Args::try_parse_from(["wayscrollshot", "--auto-save-after-idle", "0"]).is_err());
        assert!(Args::try_parse_from(["wayscrollshot", "--auto-save-after-idle", "-1s"]).is_err());
        assert!(Args::try_parse_from(["wayscrollshot", "--auto-save-after-idle", "abc"]).is_err());
    }
}
