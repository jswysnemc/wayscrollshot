//! GNOME 捕获。Mutter 没有 layer-shell 和 wlr-screencopy，选区和截图走 Shell 扩展。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;
use zbus::blocking::Connection;

use crate::types::{Region, UserCommand};

/// GNOME 捕获总开关。关闭后自动后端不再走 Shell 扩展。
pub const FEATURE_GNOME_CAPTURE_ENABLED: bool = true;

const SERVICE: &str = "org.gnome.Shell";
const PATH: &str = "/org/gnome/Shell/Extensions/WayScrollShotHelper";
const INTERFACE: &str = "org.gnome.Shell.Extensions.WayScrollShotHelper";
const SELECT_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const FRAME_TIMEOUT: Duration = Duration::from_secs(8);

/// 判断 desktop 是否为 GNOME 会话；desktop 为 `XDG_CURRENT_DESKTOP` 原文，返回是否匹配。
pub fn desktop_is_gnome(desktop: &str) -> bool {
    desktop
        .split([':', ';'])
        .any(|entry| entry.to_ascii_lowercase().contains("gnome"))
}

/// 判断当前会话是否为 GNOME；无参数。
pub fn is_gnome_session() -> bool {
    if !FEATURE_GNOME_CAPTURE_ENABLED {
        return false;
    }
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|desktop| desktop_is_gnome(&desktop))
        .unwrap_or(false)
}

/// GNOME 上用扩展选区；非 GNOME 或开关关闭时返回 None。
pub fn select_region_if_enabled() -> Option<Result<Region>> {
    if !is_gnome_session() {
        return None;
    }
    Some(select_region())
}

/// 调用扩展的 SelectArea；无参数，返回用户拖出的区域。
pub fn select_region() -> Result<Region> {
    let connection = open_bus(SELECT_TIMEOUT)?;
    ensure_helper(&connection)?;
    let reply = connection
        .call_method(Some(SERVICE), PATH, Some(INTERFACE), "SelectArea", &())
        .context("GNOME region selection failed")?;
    let (ok, x, y, w, h): (bool, i32, i32, i32, i32) = reply
        .body()
        .deserialize()
        .context("failed to decode GNOME selection")?;
    if !ok || w <= 0 || h <= 0 {
        bail!("GNOME region selection was cancelled");
    }
    Ok(Region {
        raw: format!("{x},{y} {w}x{h}"),
        x,
        y,
        w: w as u32,
        h: h as u32,
    })
}

/// 复用一条会话总线连接，按区域反复截图。
pub struct GnomeCapturer {
    connection: Connection,
    frame_seq: AtomicU64,
}

impl GnomeCapturer {
    /// 连接会话总线并确认扩展可用；无参数，返回捕获器。
    pub fn new() -> Result<Self> {
        let connection = open_bus(FRAME_TIMEOUT)?;
        ensure_helper(&connection)?;
        Ok(Self {
            connection,
            frame_seq: AtomicU64::new(0),
        })
    }

    /// 截取 region 并返回一帧 RGBA；region 为全局坐标。
    pub fn capture(&self, region: &Region) -> Result<RgbaImage> {
        let seq = self.frame_seq.fetch_add(1, Ordering::Relaxed);
        let path = frame_path(seq);
        let reply = self
            .connection
            .call_method(
                Some(SERVICE),
                PATH,
                Some(INTERFACE),
                "ScreenshotArea",
                &(
                    region.x,
                    region.y,
                    region.w as i32,
                    region.h as i32,
                    &path,
                ),
            )
            .context("GNOME ScreenshotArea failed")?;
        let (ok, used): (bool, String) = reply
            .body()
            .deserialize()
            .context("failed to decode GNOME screenshot reply")?;
        let image_path = if used.is_empty() { path.clone() } else { used };
        let outcome = if !ok {
            Err(anyhow!("GNOME Shell refused the area capture"))
        } else {
            image::open(&image_path)
                .context("failed to read GNOME screenshot")
                .map(|image| image.to_rgba8())
        };
        let _ = std::fs::remove_file(&image_path);
        if image_path != path {
            let _ = std::fs::remove_file(&path);
        }
        outcome
    }
}

/// 在没有 layer-shell 预览时，用扩展上的按钮接收保存、复制、暂停和取消。
pub struct GnomeControls {
    connection: Connection,
}

impl GnomeControls {
    /// 显示控制条并在后台把按钮动作写入 tx；region 用于避开捕获区域，返回控制句柄。
    pub fn spawn(
        region: &Region,
        tx: mpsc::Sender<UserCommand>,
    ) -> Result<thread::JoinHandle<()>> {
        let connection = open_bus(SELECT_TIMEOUT)?;
        ensure_helper(&connection)?;
        connection
            .call_method(
                Some(SERVICE),
                PATH,
                Some(INTERFACE),
                "ShowControls",
                &(region.x, region.y, region.w as i32, region.h as i32),
            )
            .context("failed to show GNOME controls")?;
        let controls = Self { connection };
        Ok(thread::spawn(move || controls.pump(tx)))
    }

    /// 阻塞等待下一次按钮；无参数，返回对应命令。
    fn wait_action(&self) -> Result<UserCommand> {
        let reply = self
            .connection
            .call_method(Some(SERVICE), PATH, Some(INTERFACE), "WaitAction", &())
            .context("GNOME control wait failed")?;
        let (action,): (String,) = reply
            .body()
            .deserialize()
            .context("failed to decode GNOME control action")?;
        match action.as_str() {
            "save" => Ok(UserCommand::Save),
            "copy" => Ok(UserCommand::Copy),
            "pause" => Ok(UserCommand::TogglePause),
            "cancel" => Ok(UserCommand::Cancel),
            other => bail!("unknown GNOME control action: {other}"),
        }
    }

    /// 隐藏控制条；无参数、无返回值。
    fn hide(&self) {
        let _ = self
            .connection
            .call_method(Some(SERVICE), PATH, Some(INTERFACE), "HideControls", &());
    }

    /// 把按钮动作转发到 tx，直到保存、复制或取消；tx 为会话命令通道。
    fn pump(self, tx: mpsc::Sender<UserCommand>) {
        loop {
            match self.wait_action() {
                Ok(command) => {
                    let done = matches!(
                        command,
                        UserCommand::Save | UserCommand::Copy | UserCommand::Cancel
                    );
                    if tx.send(command).is_err() || done {
                        break;
                    }
                }
                Err(err) => {
                    log::error!("【GNOME 捕获】【控制条】等待按钮失败: {err}");
                    let _ = tx.send(UserCommand::Cancel);
                    break;
                }
            }
        }
        self.hide();
    }
}

/// 按 timeout 连接会话总线；timeout 为单次方法调用上限，返回连接。
fn open_bus(timeout: Duration) -> Result<Connection> {
    zbus::blocking::connection::Builder::session()?
        .method_timeout(timeout)
        .build()
        .context("failed to connect to the D-Bus session bus")
}

/// 确认扩展已启用；connection 为会话总线，失败时返回安装说明。
fn ensure_helper(connection: &Connection) -> Result<()> {
    let reply = connection
        .call_method(Some(SERVICE), PATH, Some(INTERFACE), "Version", &())
        .map_err(|_| helper_missing())?;
    let (version,): (String,) = reply
        .body()
        .deserialize()
        .context("failed to decode GNOME helper version")?;
    if version.is_empty() {
        return Err(helper_missing());
    }
    log::info!("【GNOME 捕获】【扩展探测】滚动助手版本为 {version}");
    Ok(())
}

/// 返回扩展缺失时的说明；无参数。
fn helper_missing() -> anyhow::Error {
    anyhow!(
        "GNOME does not support slurp or grim. Enable the wayscrollshot scroll helper and log out once:\n  \
         gnome-extensions enable wayscrollshot-scroll-helper@snemc.org\n\
         Packages install it under /usr/share/gnome-shell/extensions/. From a source tree:\n  \
         mkdir -p ~/.local/share/gnome-shell/extensions/wayscrollshot-scroll-helper@snemc.org\n  \
         cp packaging/gnome-extension/wayscrollshot-scroll-helper@snemc.org/* \
         ~/.local/share/gnome-shell/extensions/wayscrollshot-scroll-helper@snemc.org/"
    )
}

/// 返回第 seq 帧的临时 PNG 路径；优先使用内存盘。
fn frame_path(seq: u64) -> String {
    let dir = if std::path::Path::new("/dev/shm").is_dir() {
        std::path::PathBuf::from("/dev/shm")
    } else {
        std::env::temp_dir()
    };
    dir.join(format!(
        "wayscrollshot-{}-{seq}.png",
        std::process::id()
    ))
    .to_string_lossy()
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::desktop_is_gnome;

    #[test]
    fn detects_ubuntu_gnome_desktop() {
        assert!(desktop_is_gnome("ubuntu:GNOME"));
        assert!(desktop_is_gnome("GNOME"));
        assert!(desktop_is_gnome("GNOME-Classic"));
        assert!(!desktop_is_gnome("COSMIC"));
        assert!(!desktop_is_gnome("KDE"));
        assert!(!desktop_is_gnome("sway"));
    }
}
