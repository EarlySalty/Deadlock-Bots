//! Tracing-Initialisierung für die Binaries.

use tracing_subscriber::EnvFilter;

/// Initialisiert das globale Tracing-Subscriber-Setup.
///
/// Filter kommt aus `RUST_LOG`, sonst greift der übergebene Default
/// (z. B. `"info"`). Mehrfachaufruf (Tests) ist unkritisch: der zweite
/// Aufruf wird ignoriert statt zu panicken.
pub fn init_tracing(default_filter: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}
