#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod work;

use anyhow::{Context, Result};
use clap::Parser;
use directories::ProjectDirs;
use eframe::egui;
use relay_core::Profile;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "RelayPost — carry files offline, verify their arrival")]
struct Options {
    /// Isolated profile directory; shared with the CLI by default
    #[arg(long, value_name = "DIRECTORY")]
    home: Option<PathBuf>,
}

fn main() -> Result<()> {
    let options = Options::parse();
    let home = match options.home {
        Some(home) => home,
        None => ProjectDirs::from("org", "RelayPost", "RelayPost")
            .map(|dirs| dirs.data_local_dir().to_path_buf())
            .context("Cannot locate application data. Pass --home DIRECTORY.")?,
    };
    let profile = Profile::open(home.clone())?;
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 800.0])
            .with_min_inner_size([880.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "RelayPost",
        native_options,
        Box::new(move |context| Ok(Box::new(app::RelayApp::new(context, profile, home)))),
    )
    .map_err(|error| anyhow::anyhow!("Cannot open the native application: {error}"))
}
