mod backtrace;
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
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::Local;
use human_panic::Metadata;
use log::{debug, LevelFilter};
use simplelog::{ColorChoice, CombinedLogger, TermLogger, TerminalMode, WriteLogger};

use crate::{
    injector::inject_plugins,
    panic::set_hook,
    paths::{build_config_game_binary_paths, get_bg3_plugins_dir},
    process_watcher::CallType,
};

use self::{
    config::{get_config, Config},
    popup::{display_popup, fatal_popup, MessageBoxIcon},
    process_watcher::{ProcessWatcher, Timeout},
    single_instance::SingleInstance,
    tray::AppTray,
};

#[derive(Debug, PartialEq)]
pub enum RunType {
    Watcher,
    Injector,
}

/// Process watcher entry point
pub fn run(run_type: RunType) {
    // This prohibits multiple app instances
    let _singleton = SingleInstance::new();

    let (plugins_dir, config) = setup();

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
                    debug!("Received callback for pid {pid}, now injecting");
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
}

fn setup() -> (PathBuf, Config) {
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
            fatal_popup(
                "Fatal Error",
                format!("Failed to find bg3 plugins folder: {e}"),
            );
        }
    };

    // start logger
    setup_logs(&plugins_dir).expect("Failed to set up logs");

    // get/create config
    let config = get_config(plugins_dir.join("config.toml")).expect("Failed to get config");

    if first_time {
        display_popup(
                "Finish Setup",
                format!(
                    "The plugins folder was just created at\n{}\n\nTo install plugins, place the plugin dll files inside the plugins folder.\n\nPlease also double-check `config.toml` in the plugins folder. If you installed Steam/BG3 to a non-default path, the install root in the config needs to be adjusted before launching again.",
                    plugins_dir.display()
                ),
                MessageBoxIcon::Information,
            );
        std::process::exit(0);
    }

    debug!("Got config: {config:?}");

    (plugins_dir, config)
}

fn setup_logs<P: AsRef<Path>>(plugins_dir: P) -> anyhow::Result<()> {
    let plugins_dir = plugins_dir.as_ref();

    let date = Local::now();
    let date = date.format("%Y-%m-%d").to_string();

    let logs_dir = plugins_dir.join("logs");

    let log_path = logs_dir.join(format!("native-mod-launcher {date}.log"));

    let file = if log_path.exists() {
        match OpenOptions::new().append(true).open(log_path) {
            Ok(v) => v,
            Err(e) => {
                fatal_popup("Fatal Error", format!("Failed to open log file: {e}"));
            }
        }
    } else {
        match File::create(log_path) {
            Ok(v) => v,
            Err(e) => {
                fatal_popup("Fatal Error", format!("Failed to create log file: {e}"));
            }
        }
    };

    // enable logging
    CombinedLogger::init(vec![
        TermLogger::new(
            if cfg!(debug_assertions) {
                LevelFilter::Debug
            } else {
                LevelFilter::Info
            },
            simplelog::Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        // save log to plugins dir
        WriteLogger::new(LevelFilter::Info, simplelog::Config::default(), file),
    ])?;

    Ok(())
}
