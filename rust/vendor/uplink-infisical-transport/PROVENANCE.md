Der bestehende Uplink-Transport ist als portable, unveränderte Client-Kopie
eingebunden, entsprechend Deadlock-Brain/rust/vendor/uplink-infisical-transport.
Quelle: uplink/crates/uplink-infisical-transport/src/lib.rs. Diese Crate enthält
ausschließlich die Prüfung des geschützten Unixsocket-Pfads und den ClientBuilder;
keine eigene Bridge, keinen Secret-Bootstrap und keine Providerkonfiguration.
