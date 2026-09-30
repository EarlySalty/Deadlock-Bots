//! Fester Einstiegspunkt für Registry-Prüfung und systemd-überwachte Aktivierung.
use std::path::PathBuf;

fn run() -> Result<(), &'static str> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 2 && args[0] == "--check-registry" {
        let path = PathBuf::from(&args[1]);
        if !path.is_absolute() { return Err("Die Registry muss absolut angegeben werden."); }
        let registry = dl_core::admin_config::Registry::load(&path).map_err(|_| "Ungültige oder nicht lesbare Registry.")?;
        for target in registry.bots {
            if !target.enabled { println!("{}: nicht verbunden", target.id); continue; }
            let snapshot = target.snapshot().map_err(|_| "Eine aktivierte Bot-Konfiguration konnte nicht validiert werden.")?;
            let services = dl_core::admin_config_activation::observe(&target)
                .map_err(|_| "Der registrierte Dienststatus konnte nicht geprüft werden.")?;
            println!("{}: Schema gültig, Revision {}, {} von {} Diensten mit explizitem TOML-Startpfad", target.id, snapshot.revision,
                services.iter().filter(|service| service.state == "active" && service.config_connected).count(), services.len());
        }
        return Ok(());
    }
    if args.len() != 6 || args[0] != "--registry" || args[2] != "--bot" || args[4] != "--revision" {
        return Err("Aufruf: dl-config-apply --check-registry <Datei> | --registry <Datei> --bot <ID> --revision <SHA256>");
    }
    let registry = PathBuf::from(&args[1]);
    if !registry.is_absolute() { return Err("Die Registry muss absolut angegeben werden."); }
    dl_core::admin_config_activation::apply(&registry, &args[3], &args[5])
        .map_err(|_| "Aktivierung nicht bestätigt; den Status im Admin-Dashboard prüfen.")
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => { eprintln!("{message}"); std::process::ExitCode::FAILURE }
    }
}
