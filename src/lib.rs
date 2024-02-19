mod backtrace;
mod cli;
mod config;
mod helpers;
mod injector;
mod panic;
mod paths;
mod popup;
mod process_watcher;
mod single_instance;
mod tray;

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use clap::Parser;
use eyre::{Context, Result};
use human_panic::Metadata;
use tracing::{error, level_filters::LevelFilter, trace};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

use cli::Args;
use config::{get_config, Config};
use injector::inject_plugins;
use panic::set_hook;
use paths::{build_config_game_binary_paths, get_bg3_plugins_dir};
use popup::{display_popup, fatal_popup, MessageBoxIcon};
use process_watcher::CallType;
use process_watcher::{ProcessWatcher, Timeout};
use single_instance::SingleInstance;
use tray::AppTray;
use windows::Win32::System::Console::{
    AllocConsole, GetStdHandle, SetConsoleMode, ENABLE_PROCESSED_OUTPUT,
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, ENABLE_WRAP_AT_EOL_OUTPUT, STD_OUTPUT_HANDLE,
};

#[derive(Debug, PartialEq)]
pub enum RunType {
    Watcher,
    Injector,
}

/// Process watcher entry point
pub fn run(run_type: RunType) -> Result<()> {
    // This prohibits multiple app instances
    let _singleton = SingleInstance::new();

    let args = Args::parse();

    let (plugins_dir, config, _guard) = setup(&args)?;

    let (bg3, bg3_dx11) = build_config_game_binary_paths(&config);

    let (polling_rate, timeout, oneshot) = if run_type == RunType::Watcher {
        // watcher tool
        (Duration::from_secs(2), Timeout::None, false)
    } else {
        // injector tool
        (
            Duration::from_secs(1),
            Timeout::Duration(Duration::from_secs(10)),
            true,
        )
    };

    let (waiter, stop_token) =
        ProcessWatcher::new(&[bg3, bg3_dx11], polling_rate, timeout, oneshot).run(
        move |call| match call {
                CallType::Pid(pid) => {
                    trace!("Received callback for pid {pid}, now injecting");
                    inject_plugins(pid, &plugins_dir, &config).unwrap();
                }

                // only fires with injector
                CallType::Timeout => {
                    fatal_popup(
                        "Fatal Error",
                        "Game process was not found.\n\nThis can happen for 1 of 2 reasons:\n\nEither the game isn't running, so this tool timed out waiting for it\n\nOr the game wasn't detected because your `install_root` config value isn't correct\n\nIn rare cases, it could be that the program doesn't have permission to open the game process, so it skips it. In such a case, you should run this as admin (only as a last resort; in normal cases this is not needed)",
                    );
                }
            }
        );

    // tray
    if run_type == RunType::Watcher {
        AppTray::start(stop_token);
    }

    waiter.wait();

    Ok(())
}

fn setup(args: &Args) -> Result<(PathBuf, Config, Option<WorkerGuard>)> {
    // Nicely print any panic messages to the user
    set_hook(Metadata {
        name: env!("CARGO_PKG_NAME").into(),
        version: env!("CARGO_PKG_VERSION").into(),
        authors: "Cherry".into(),
        homepage: "https://github.com/MolotovCherry/Yet-Another-BG3-Native-Mod-Loader".into(),
    });

    let (first_time, plugins_dir) = match get_bg3_plugins_dir() {
        Ok(v) => v,
        Err(e) => {
            error!("failed to find plugins_dir: {e}");
            fatal_popup("Fatal Error", "Failed to find bg3 plugins folder");
        }
    };

    // start logger
    let worker_guard = setup_logs(&plugins_dir, args).context("Failed to set up logs")?;

    // get/create config
    let config = get_config(plugins_dir.join("config.toml")).context("Failed to get config")?;

    if first_time {
        display_popup(
                "Finish Setup",
                format!(
                    "The plugins folder was just created at\n{}\n\nTo install plugins, place the plugin dll files inside the plugins folder.\n\nPlease also double-check `config.toml` in the plugins folder. install_root in the config likely needs to be adjusted to the correct path.",
                    plugins_dir.display()
                ),
                MessageBoxIcon::Information,
            );
        std::process::exit(0);
    }

    trace!("Got config: {config:?}");

    Ok((plugins_dir, config, worker_guard))
}

fn setup_logs<P: AsRef<Path>>(plugins_dir: P, args: &Args) -> Result<Option<WorkerGuard>> {
    let mut worker_guard: Option<WorkerGuard> = None;

    if cfg!(debug_assertions) || args.cli {
        if cfg!(not(debug_assertions)) {
            unsafe {
                AllocConsole()?;

                let handle = GetStdHandle(STD_OUTPUT_HANDLE)?;
                SetConsoleMode(
                    handle,
                    ENABLE_PROCESSED_OUTPUT
                        | ENABLE_WRAP_AT_EOL_OUTPUT
                        | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
                )?;
            }
        }

        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_env("YABG3ML_LOG"))
            .without_time()
            .init();
    } else {
        let plugins_dir = plugins_dir.as_ref();
        let logs_dir = plugins_dir.join("logs");

        let file_appender = tracing_appender::rolling::daily(logs_dir, "ya-native-mod-loader");
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

        worker_guard = Some(_guard);
        tracing_subscriber::fmt()
            .with_max_level(LevelFilter::DEBUG)
            .with_writer(non_blocking)
            .with_ansi(false)
            .init();
    }

    Ok(worker_guard)
}
