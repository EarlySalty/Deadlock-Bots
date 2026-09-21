//! Tracing-Initialisierung für die Binaries.

use tracing_subscriber::EnvFilter;

/// Initialisiert das globale Tracing-Subscriber-Setup.
///
/// Verwendet ausschließlich den übergebenen, beim Configladen geprüften Filter
/// (z. B. `"info"`). Mehrfachaufruf (Tests) ist unkritisch: der zweite
/// Aufruf wird ignoriert statt zu panicken.
pub fn init_tracing(default_filter: &str) {
    let filter = EnvFilter::new(default_filter);
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}
