use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;

use crate::capture::{
    capture_frame, read_region_from_stdin, region_from_slurp_output, select_region,
};
use crate::cli::{Algorithm, Args};
use crate::output::{copy_to_clipboard, save_image};
use crate::overlay::LayerShellOverlay;
use crate::region_overlay::RegionOverlay;
use crate::stitch::{build_preview, init_opencv_runtime, MatchConfig, StitchOutcome, Stitcher};
use crate::types::{Control, LayerMessage, Region, StitchState, UserCommand};

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

    let mut region_overlay = if args.no_border {
        None
    } else {
        Some(RegionOverlay::new(region.clone())?)
    };

    let control = Arc::new(Control::new());
    let state = Arc::new(Mutex::new(StitchState::default()));

    let (command_tx, command_rx) = mpsc::channel();

    let mut layer_overlay = if args.no_preview {
        None
    } else {
        Some(LayerShellOverlay::new(
            command_tx.clone(),
            region.clone(),
            args.preview_width,
        )?)
    };

    let preview_tx = layer_overlay.as_ref().and_then(|o| o.sender());
    let algorithm = args.algorithm;
    let worker = spawn_capture_worker(
        region,
        control.clone(),
        state.clone(),
        preview_tx,
        args.preview_width,
        algorithm,
    );

    let result = run_session(
        control,
        state,
        worker,
        &mut layer_overlay,
        command_rx,
        &args,
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

/// Handles overlay commands and finalization while capture worker is running.
fn run_session(
    control: Arc<Control>,
    state: Arc<Mutex<StitchState>>,
    worker: thread::JoinHandle<()>,
    layer_overlay: &mut Option<LayerShellOverlay>,
    command_rx: mpsc::Receiver<UserCommand>,
    args: &Args,
) -> Result<()> {
    let mut paused = false;

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
                        log::info!("Saved to {}", path.display());
                        control.stop();
                        break;
                    }
                    Err(err) => {
                        log::error!("Save failed: {err}");
                    }
                }
            }
            UserCommand::Copy => match take_snapshot(&state).and_then(copy_to_clipboard) {
                Ok(()) => {
                    log::info!("Copied to clipboard");
                    control.stop();
                    break;
                }
                Err(err) => {
                    log::error!("Copy failed: {err}");
                }
            },
            UserCommand::Cancel => {
                control.stop();
                break;
            }
        }
    }

    control.stop();
    if let Some(ref mut overlay) = layer_overlay {
        overlay.stop();
    }
    let _ = worker.join();
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

/// Spawns background capture + stitching worker.
fn spawn_capture_worker(
    region: Region,
    control: Arc<Control>,
    state: Arc<Mutex<StitchState>>,
    preview_tx: Option<mpsc::Sender<LayerMessage>>,
    preview_width: u32,
    algorithm: Algorithm,
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
        let mut last_capture_end: Option<Instant> = None;
        let mut last_signature: Option<Vec<u8>> = None;

        while control.is_running() {
            if control.is_paused() {
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

            match capture_frame(&region) {
                Ok(frame) => {
                    last_capture_end = Some(Instant::now());

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
                            continue;
                        }
                    }
                    last_signature = Some(signature);

                    let outcome = stitcher.push_frame(frame);
                    match outcome {
                        StitchOutcome::FirstFrame => {
                            apply_state_update(
                                &state,
                                &stitcher,
                                "First frame captured".to_string(),
                                preview_tx.as_ref(),
                                preview_width,
                            );
                        }
                        StitchOutcome::Appended { added } => {
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
                }
                Err(err) => {
                    update_status(
                        &state,
                        "Capture error".to_string(),
                        Some(&stitcher),
                        None,
                        Some(err.to_string()),
                    );
                }
            }
        }
    })
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
