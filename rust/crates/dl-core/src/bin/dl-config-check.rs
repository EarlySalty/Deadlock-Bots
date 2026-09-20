use std::{ffi::OsString, process::ExitCode};

use dl_core::Config;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let valid_shape = args.is_empty()
        || (args.len() == 2 && args[0] == "--config")
        || (args.len() == 1 && args[0].to_string_lossy().starts_with("--config="));
    if !valid_shape {
        eprintln!("Aufruf: dl-config-check [--config PFAD]");
        return ExitCode::from(2);
    }
    match Config::from_process() {
        Ok(config) => {
            println!(
                "Config-Syntax und Schema gültig. master_broker_port={} changelog_port={}. \
                 Dienstfunktion und weitere Modulkonfigurationen wurden nicht geprüft.",
                config.ports.master_broker, config.ports.changelog_api
            );
            ExitCode::SUCCESS
        }
        Err(dl_core::ConfigError::Arguments) => {
            eprintln!("Aufruf: dl-config-check [--config PFAD]");
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
