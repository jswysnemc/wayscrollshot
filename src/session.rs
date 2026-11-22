use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use std::io::{self, BufRead, IsTerminal};

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;

use crate::capture::{read_region_from_stdin, region_from_slurp_output, select_region, Capturer};
use crate::cli::{Algorithm, Args};
use crate::idle_finish::IdleAutoFinish;
use crate::output::{copy_to_clipboard, save_image};
use crate::overlay::LayerShellOverlay;
use crate::region_overlay::RegionOverlay;
use crate::stitch::{build_preview, init_opencv_runtime, MatchConfig, StitchOutcome, Stitcher};
use crate::types::{Control, LayerMessage, Region, StitchState, UserCommand};

// ===== 新增 import =====
use crate::capture::gnome::GnomeControls;

const CAPTURE_INTERVAL: Duration = Duration::from_millis(45);
const SIGNATURE_COLS: u32 = 18;
const SIGNATURE_ROWS: u32 = 24;
const DUPLICATE_AVG_DIFF: f32 = 1.1;
const DUPLICATE_MAX_DIFF: u8 = 4;

/// Runs one interactive capture session from region selection to final action.
pub fn run(args: Args) -> Result<()> {
    if !is_wayland_session() {
        bail!("Wayland session required (X11 not supported)");
    }

    let region = resolve_region(&args)?;
    log::info!(
        "Capture region: {},{} {}x{}",
        region.x,
        region.y,
        region.w,
        region.h
    );

    let capturer = Capturer::new(args.backend)?;

    let mut region_overlay = start_region_border(&args, &region);

    let control = Arc::new(Control::new());
    let state = Arc::new(Mutex::new(StitchState::default()));

    let (command_tx, command_rx) = mpsc::channel();
    let auto_save_idle = args.effective_auto_save_idle();

    let mut layer_overlay = start_preview_overlay(&args, &region, command_tx.clone());
    let fallback_controls = attach_fallback_controls(
        layer_overlay.is_none(),
        &args,
        &region,
        command_tx.clone(),
        auto_save_idle.is_some(),
    );
    let fail_on_action_error =
        layer_overlay.is_none() && !fallback_controls.has_interactive_control;

    let preview_tx = layer_overlay.as_ref().and_then(|o| o.sender());
    let worker_command_tx = auto_save_idle.map(|_| command_tx);
    let auto_finish_command = if args.clipboard {
        UserCommand::Copy
    } else {
        UserCommand::Save
    };
    let algorithm = args.algorithm;
    let worker = spawn_capture_worker(
        capturer,
        region,
        control.clone(),
        state.clone(),
        preview_tx,
        args.preview_width,
        algorithm,
        auto_save_idle,
        auto_finish_command,
        worker_command_tx,
    );

    let result = run_session(
        control,
        state,
        worker,
        &mut layer_overlay,
        command_rx,
        &args,
        fail_on_action_error,
    );

    if let Some(ref mut overlay) = region_overlay {
        overlay.stop();
    }
    result
}

fn resolve_region(args: &Args) -> Result<Region> {
    match args.slurp_output() {
        Some(raw) if raw.trim() == "-" => {
            read_region_from_stdin().context("failed to read slurp selection from stdin")
        }
        Some(raw) => region_from_slurp_output(&raw).context("invalid slurp selection"),
        None => select_region().context("slurp selection failed"),
    }
}

/// 处理浮层或自动完成指令，直到会话结束并回收捕获线程
///
/// `control` 为运行控制标志，`state` 为拼接状态，`worker` 为捕获线程句柄，
/// `layer_overlay` 为预览浮层，`command_rx` 为指令接收端，`args` 为命令行参数，
/// `fail_on_action_error` 表示保存或复制失败时是否直接返回错误退出；返回会话执行结果
fn run_session(
    control: Arc<Control>,
    state: Arc<Mutex<StitchState>>,
    worker: thread::JoinHandle<()>,
    layer_overlay: &mut Option<LayerShellOverlay>,
    command_rx: mpsc::Receiver<UserCommand>,
    args: &Args,
    fail_on_action_error: bool,
) -> Result<()> {
    let mut paused = false;
    let mut session_error: Option<anyhow::Error> = None;

    // 1. 持续消费用户或空闲自动完成发出的控制指令
    while let Ok(command) = command_rx.recv() {
        match command {
            UserCommand::TogglePause => {
                control.toggle_pause();
                paused = !paused;
                if let Some(ref overlay) = layer_overlay {
                    overlay.send(LayerMessage::Paused(paused));
                }
            }
            UserCommand::Save => {
                match take_snapshot(&state).and_then(|img| save_image(img, args.output.clone())) {
                    Ok(path) => {
                        log::info!("【长截图会话】【保存完成】已保存到 {}", path.display());
                        control.stop();
                        break;
                    }
                    Err(err) => {
                        log::error!("【长截图会话】【保存失败】保存截图失败: {err}");
                        if fail_on_action_error {
                            session_error = Some(err);
                            control.stop();
                            break;
                        }
                    }
                }
            }
            UserCommand::Copy => match take_snapshot(&state).and_then(copy_to_clipboard) {
                Ok(()) => {
                    log::info!("【长截图会话】【复制完成】已复制到剪贴板");
                    control.stop();
                    break;
                }
                Err(err) => {
                    log::error!("【长截图会话】【复制失败】复制到剪贴板失败: {err}");
                    if fail_on_action_error {
                        session_error = Some(err);
                        control.stop();
                        break;
                    }
                }
            },
            UserCommand::Cancel => {
                control.stop();
                break;
            }
        }
    }

    // 2. 停止工作线程与预览浮层并返回结果
    control.stop();
    if let Some(ref mut overlay) = layer_overlay {
        overlay.stop();
    }
    let _ = worker.join();
    if let Some(err) = session_error {
        return Err(err);
    }
    Ok(())
}

/// Returns the latest stitched image snapshot.
fn take_snapshot(state: &Arc<Mutex<StitchState>>) -> Result<Arc<RgbaImage>> {
    let state = state.lock().expect("state lock");
    state
        .full_image
        .clone()
        .ok_or_else(|| anyhow!("no frames captured yet"))
}

/// Detects whether current desktop session is Wayland.
fn is_wayland_session() -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return true;
    }
    matches!(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        Some("wayland")
    )
}

/// 启动后台截屏与拼接工作线程，并在启用空闲自动完成时负责触发最终保存或复制
///
/// `capturer` 为截屏后端，`region` 为选区，`control` 为运行控制标志，`state` 为共享拼接状态，
/// `preview_tx` 为预览通道，`preview_width` 为预览宽度，`algorithm` 为拼接算法，
/// `auto_save_idle` 为空闲自动完成时长，`auto_finish_command` 为自动完成指令，
/// `command_tx` 为会话指令发送端；返回工作线程句柄
#[allow(clippy::too_many_arguments)]
fn spawn_capture_worker(
    capturer: Capturer,
    region: Region,
    control: Arc<Control>,
    state: Arc<Mutex<StitchState>>,
    preview_tx: Option<mpsc::Sender<LayerMessage>>,
    preview_width: u32,
    algorithm: Algorithm,
    auto_save_idle: Option<Duration>,
    auto_finish_command: UserCommand,
    command_tx: Option<mpsc::Sender<UserCommand>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let config = match algorithm {
            Algorithm::OpenCvOrb => MatchConfig {
                min_overlap: 120,
                accept_diff: 3.5,
                min_append: 10,
                approx_diff: 1.0,
                match_width: preview_width.max(200),
                algorithm,
            },
            _ => MatchConfig {
                min_overlap: 100,
                accept_diff: 5.0,
                min_append: 15,
                approx_diff: 1.0,
                match_width: preview_width.max(200),
                algorithm,
            },
        };
        if matches!(algorithm, Algorithm::OpenCvOrb) {
            init_opencv_runtime();
        }
        let mut stitcher = Stitcher::new(config);
        let mut idle_finish = IdleAutoFinish::new(auto_save_idle);
        let mut last_capture_end: Option<Instant> = None;
        let mut last_signature: Option<Vec<u8>> = None;

        while control.is_running() {
            if control.is_paused() {
                idle_finish.on_paused();
                update_status(&state, "Paused".to_string(), None, None, None);
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }

            if let Some(last) = last_capture_end {
                let elapsed = last.elapsed();
                if elapsed < CAPTURE_INTERVAL {
                    std::thread::sleep(CAPTURE_INTERVAL - elapsed);
                }
            }

            match capturer.capture(&region) {
                Ok(frame) => {
                    let now = Instant::now();
                    last_capture_end = Some(now);

                    let signature = frame_signature(&frame, SIGNATURE_COLS, SIGNATURE_ROWS);
                    if let Some(previous) = last_signature.as_ref() {
                        if is_duplicate_signature(previous, &signature) {
                            update_status(
                                &state,
                                "Waiting for scroll".to_string(),
                                Some(&stitcher),
                                None,
                                None,
                            );
                            if maybe_trigger_idle_finish(
                                &mut idle_finish,
                                now,
                                &command_tx,
                                &auto_finish_command,
                            ) {
                                break;
                            }
                            continue;
                        }
                    }
                    last_signature = Some(signature);

                    let outcome = stitcher.push_frame(frame);
                    match outcome {
                        StitchOutcome::FirstFrame => {
                            idle_finish.on_first_frame(now);
                            apply_state_update(
                                &state,
                                &stitcher,
                                "First frame captured".to_string(),
                                preview_tx.as_ref(),
                                preview_width,
                            );
                        }
                        StitchOutcome::Appended { added } => {
                            idle_finish.on_appended(now);
                            apply_state_update(
                                &state,
                                &stitcher,
                                format!("Appended {added} px"),
                                preview_tx.as_ref(),
                                preview_width,
                            );
                        }
                        StitchOutcome::NoProgress => {
                            update_status(
                                &state,
                                "No scroll detected".to_string(),
                                Some(&stitcher),
                                None,
                                None,
                            );
                        }
                        StitchOutcome::NoMatch => {
                            update_status(
                                &state,
                                "No overlap match".to_string(),
                                Some(&stitcher),
                                None,
                                None,
                            );
                        }
                    }

                    if maybe_trigger_idle_finish(
                        &mut idle_finish,
                        Instant::now(),
                        &command_tx,
                        &auto_finish_command,
                    ) {
                        break;
                    }
                }
                Err(err) => {
                    update_status(
                        &state,
                        "Capture error".to_string(),
                        Some(&stitcher),
                        None,
                        Some(err.to_string()),
                    );
                    if maybe_trigger_idle_finish(
                        &mut idle_finish,
                        Instant::now(),
                        &command_tx,
                        &auto_finish_command,
                    ) {
                        break;
                    }
                }
            }
        }
    })
}

/// 在满足空闲超时条件时发送自动完成指令
///
/// `idle_finish` 为空闲控制器，`now` 为当前时间戳，`command_tx` 为会话指令发送端，
/// `auto_finish_command` 为需要触发的最终指令；返回是否已触发自动完成
fn maybe_trigger_idle_finish(
    idle_finish: &mut IdleAutoFinish,
    now: Instant,
    command_tx: &Option<mpsc::Sender<UserCommand>>,
    auto_finish_command: &UserCommand,
) -> bool {
    if !idle_finish.should_trigger(now, false) {
        return false;
    }

    log::info!(
        "【长截图会话】【空闲自动完成】滚动已停止，触发自动完成指令: {:?}",
        auto_finish_command
    );
    if let Some(tx) = command_tx {
        let _ = tx.send(auto_finish_command.clone());
    }
    true
}

/// Applies a successful stitch update and publishes preview frame.
fn apply_state_update(
    state: &Arc<Mutex<StitchState>>,
    stitcher: &Stitcher,
    message: String,
    preview_tx: Option<&mpsc::Sender<LayerMessage>>,
    preview_width: u32,
) {
    let preview = stitcher
        .full_image()
        .map(|img| build_preview(img.as_ref(), preview_width));
    if let (Some(tx), Some(preview)) = (preview_tx, preview.as_ref()) {
        let _ = tx.send(LayerMessage::Preview(preview.clone()));
    }
    let mut st = state.lock().expect("state lock");
    st.full_image = stitcher.full_image();
    st.preview = preview;
    st.stats = stitcher.stats();
    st.last_message = message;
    st.last_error = None;
    st.revision = st.revision.wrapping_add(1);
}

/// Updates session status/message without appending new content.
fn update_status(
    state: &Arc<Mutex<StitchState>>,
    message: String,
    stitcher: Option<&Stitcher>,
    preview: Option<crate::types::PreviewImage>,
    error: Option<String>,
) {
    let mut st = state.lock().expect("state lock");
    if let Some(stitcher) = stitcher {
        st.full_image = stitcher.full_image();
        st.stats = stitcher.stats();
    }
    if preview.is_some() {
        st.preview = preview;
    }
    st.last_message = message;
    st.last_error = error;
    st.revision = st.revision.wrapping_add(1);
}

fn frame_signature(frame: &RgbaImage, cols: u32, rows: u32) -> Vec<u8> {
    let width = frame.width().max(1);
    let height = frame.height().max(1);
    let cols = cols.max(1);
    let rows = rows.max(1);
    let mut signature = Vec::with_capacity((cols * rows) as usize);

    for row in 0..rows {
        let y = ((row * height) / rows).min(height - 1);
        for col in 0..cols {
            let x = ((col * width) / cols).min(width - 1);
            let pixel = frame.get_pixel(x, y);
            let gray =
                (0.299 * pixel[0] as f32 + 0.587 * pixel[1] as f32 + 0.114 * pixel[2] as f32) as u8;
            signature.push(gray);
        }
    }

    signature
}

fn is_duplicate_signature(previous: &[u8], current: &[u8]) -> bool {
    if previous.len() != current.len() || previous.is_empty() {
        return false;
    }

    let mut sum = 0f32;
    let mut max_diff = 0u8;

    for (&a, &b) in previous.iter().zip(current.iter()) {
        let diff = a.abs_diff(b);
        max_diff = max_diff.max(diff);
        sum += diff as f32;
    }

    let avg = sum / previous.len() as f32;
    avg <= DUPLICATE_AVG_DIFF && max_diff <= DUPLICATE_MAX_DIFF
}

/// layer-shell 不可用时保存回退控制句柄
struct FallbackControls {
    _thread: Option<thread::JoinHandle<()>>,
    _tx: Option<mpsc::Sender<UserCommand>>,
    has_interactive_control: bool,
}

/// 尝试绘制区域边框；失败时继续捕获。
///
/// `args` 为命令行参数，`region` 为选区；返回边框句柄。
fn start_region_border(args: &Args, region: &Region) -> Option<RegionOverlay> {
    if args.no_border {
        return None;
    }
    match RegionOverlay::new(region.clone()) {
        Ok(overlay) => Some(overlay),
        Err(err) => {
            log::info!("【区域边框】【启动】layer-shell 不可用: {err}");
            None
        }
    }
}

/// 尝试启动预览浮层；GNOME 等没有 layer-shell 的合成器返回 None。
///
/// `command_tx` 接收浮层按钮；返回浮层句柄。
fn start_preview_overlay(
    args: &Args,
    region: &Region,
    command_tx: mpsc::Sender<UserCommand>,
) -> Option<LayerShellOverlay> {
    if args.no_preview {
        return None;
    }
    match LayerShellOverlay::new(command_tx, region.clone(), args.preview_width) {
        Ok(overlay) => Some(overlay),
        Err(err) => {
            log::info!("【预览浮层】【启动】layer-shell 不可用: {err}");
            None
        }
    }
}

/// 没有预览浮层时按模式改用 GNOME 按钮、终端命令或空闲自动完成
///
/// `needed` 表示预览浮层未启动，`args` 为命令行参数，`region` 为选区，
/// `tx` 为会话命令通道，`auto_save_enabled` 表示是否启用了空闲自动完成；返回需保持存活的句柄
fn attach_fallback_controls(
    needed: bool,
    args: &Args,
    region: &Region,
    tx: mpsc::Sender<UserCommand>,
    auto_save_enabled: bool,
) -> FallbackControls {
    if !needed {
        return FallbackControls {
            _thread: None,
            _tx: None,
            has_interactive_control: false,
        };
    }

    // 1. 仅在未显式禁用预览时，在 GNOME 会话下启用扩展控制条
    if !args.no_preview && crate::capture::gnome::is_gnome_session() {
        match GnomeControls::spawn(region, tx.clone()) {
            Ok(thread) => {
                return FallbackControls {
                    _thread: Some(thread),
                    _tx: None,
                    has_interactive_control: true,
                };
            }
            Err(err) => {
                log::error!("【GNOME 捕获】【控制条】启动失败: {err}");
            }
        }
    }

    // 2. 在交互式终端且未占用 stdin 读取选区时，允许通过终端命令提前结束或暂停
    let stdin_used_for_region = args.slurp_output().as_deref() == Some("-");
    if !stdin_used_for_region && io::stdin().is_terminal() {
        if auto_save_enabled {
            eprintln!(
                "Preview overlay disabled. Scrolling will auto-finish after idle, or type a command and press Enter: s save, c copy, p pause, q cancel"
            );
        } else {
            eprintln!(
                "Preview overlay is unavailable. Type a command and press Enter: s save, c copy, p pause, q cancel"
            );
        }
        return FallbackControls {
            _thread: Some(spawn_stdin_commands(tx)),
            _tx: None,
            has_interactive_control: true,
        };
    }

    // 3. 非交互环境下若启用了空闲自动完成，直接交由工作线程在空闲后触发保存或复制
    if auto_save_enabled {
        log::info!(
            "【预览浮层】【非交互模式】已禁用预览浮层，将在滚动空闲后自动完成并退出"
        );
        return FallbackControls {
            _thread: None,
            _tx: None,
            has_interactive_control: false,
        };
    }

    log::error!(
        "【预览浮层】【回退】没有可用的控制界面。在终端中运行，或启用 GNOME 滚动助手。"
    );
    FallbackControls {
        _thread: None,
        _tx: Some(tx),
        has_interactive_control: false,
    }
}

/// 从标准输入读取单行命令并转发；tx 为会话命令通道，返回线程句柄。
fn spawn_stdin_commands(tx: mpsc::Sender<UserCommand>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else {
                break;
            };
            let command = match line.trim() {
                "s" | "S" => UserCommand::Save,
                "c" | "C" => UserCommand::Copy,
                "p" | "P" => UserCommand::TogglePause,
                "q" | "Q" => UserCommand::Cancel,
                _ => continue,
            };
            let done = matches!(
                command,
                UserCommand::Save | UserCommand::Copy | UserCommand::Cancel
            );
            if tx.send(command).is_err() || done {
                break;
            }
        }
    })
}
