#![allow(dead_code)]
mod app;
mod audio;
mod session;
mod tui;

use anyhow::{Context, Result};
use app::App;
use audio::decoder::decode_file;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, LeaveAlternateScreen},
};
use std::io::stdout;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    // Set up file logging (never write to stdout — ratatui owns it)
    let log_dir = dirs_or_fallback();
    let log_path = log_dir.join("screwdriver.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .unwrap_or_else(|_| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("screwdriver.log")
                .expect("failed to open log file")
        });
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with_writer(std::sync::Mutex::new(log_file))
        .with_ansi(false)
        .init();

    // Panic hook: restore terminal before printing the panic
    std::panic::set_hook(Box::new(|info| {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
        eprintln!("screwdriver panicked: {info}");
    }));

    // Collect file arguments
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        anyhow::bail!("Usage: screwdriver <file> [file...]\nSupported: WAV, MP3, FLAC, OGG");
    }

    let files: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();
    for f in &files {
        if !f.exists() {
            anyhow::bail!("File not found: {}", f.display());
        }
    }

    // Decode the first file to start
    let decoded = decode_file(&files[0])
        .with_context(|| format!("Failed to decode {}", files[0].display()))?;

    tracing::info!("starting screwdriver with {} file(s)", files.len());

    let mut app = App::new(files, decoded)?;
    app.run()?;

    Ok(())
}

fn dirs_or_fallback() -> PathBuf {
    // Try ~/.local/share/screwdriver, fall back to current dir
    if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("screwdriver");
        let _ = std::fs::create_dir_all(&p);
        if p.exists() {
            return p;
        }
    }
    PathBuf::from(".")
}
