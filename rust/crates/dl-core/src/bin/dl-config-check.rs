use std::{ffi::OsString, path::PathBuf, process::ExitCode};
use dl_core::bot_config::{BotConfig, DEFAULT_CONFIG_PATH};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let path = match args.next() {
        None => PathBuf::from(DEFAULT_CONFIG_PATH),
        Some(flag) if flag == OsString::from("--config") => match args.next() {
            Some(path) => PathBuf::from(path),
            None => { eprintln!("Pfad nach --config fehlt"); return ExitCode::from(2); }
        },
        Some(_) => { eprintln!("Aufruf: dl-config-check [--config PFAD]"); return ExitCode::from(2); }
    };
    if args.next().is_some() {
        eprintln!("Aufruf: dl-config-check [--config PFAD]");
        return ExitCode::from(2);
    }
    match BotConfig::load(path) {
        Ok(_) => {
            println!("Config-Syntax und Schema gültig. Laufzeitintegration und Dienstfunktion wurden nicht geprüft.");
            ExitCode::SUCCESS
        },
        Err(error) => { eprintln!("{error}"); ExitCode::FAILURE }
    }
}
