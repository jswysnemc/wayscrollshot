mod capture;
mod cli;
mod constants;
mod opencv_compat;
mod output;
mod overlay;
mod region_overlay;
mod session;
mod stitch;
mod types;

use anyhow::Result;

use crate::cli::Args;

/// Program entrypoint.
fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse_args();
    session::run(args)
}
