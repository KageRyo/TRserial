#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod accounts;
mod app;
mod backend;
mod codes;
mod dashboard;
mod history;
mod mark;
mod platform;
mod settings;
#[cfg(test)]
mod testing;
mod ui;
mod update;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "TRserial：跑Online（TalesRunner）桌面序號兌換工具")]
struct Arguments {
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    config: Option<PathBuf>,
    /// 在全新資料目錄執行離線原生視窗測試，不連線兌換網站。
    #[arg(long, requires = "data_dir")]
    smoke_test: bool,
}
fn main() -> anyhow::Result<()> {
    platform::attach_parent_console();
    trserial::init_logging();
    let args = Arguments::parse();
    let dir = args
        .data_dir
        .unwrap_or(trserial::config::default_data_dir()?);
    let config = args.config.unwrap_or_else(|| dir.join("config.toml"));
    // Resolve before a Linux update replaces the file and the path goes stale.
    let executable = std::env::current_exe().and_then(std::fs::canonicalize);
    let backend = backend::Backend::start(dir, config, args.smoke_test);
    app::run(backend.clone(), args.smoke_test);
    if let backend::UpdateState::Ready { relaunch: true } = backend.snapshot().update {
        // The window closes only after the backend released the data-directory
        // lock, so the new version can take it over with the same arguments.
        std::process::Command::new(executable?)
            .args(std::env::args_os().skip(1))
            .spawn()
            .map_err(|error| anyhow::anyhow!("無法啟動新版 TRserial：{error}"))?;
    }
    if args.smoke_test {
        let result = backend.snapshot();
        if let Some(error) = result.error {
            anyhow::bail!(error);
        }
        anyhow::ensure!(result.smoke_complete, "原生視窗測試未完成");
    }
    Ok(())
}
