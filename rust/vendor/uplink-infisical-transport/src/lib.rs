//! Infisical ausschließlich über eine geschützte lokale Unix-Gegenstelle.
use std::{
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Component, Path, PathBuf},
};

pub const BASE_URL: &str = "http://infisical.local";
pub const DEFAULT_SOCKET: &str = "/run/uplink-infisical/api.sock";

/// Der Socketname ist durch vertrauenswürdige Verzeichnisse gegen Austausch
/// geschützt. Die Bridge prüft zusätzlich die tatsächliche Client-UID. In
/// Produktion ist der erwartete Besitzer root; isolierte Tests dürfen ihren
/// eigenen Benutzer in einem privaten Verzeichnis verwenden.
pub fn client_builder(
    socket_path: &Path,
    expected_owner_uid: u32,
) -> Result<reqwest::ClientBuilder, &'static str> {
    validate_socket(socket_path, expected_owner_uid)?;
    Ok(reqwest::Client::builder()
        .unix_socket(socket_path.to_path_buf())
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none()))
}

pub fn validate_socket(path: &Path, owner: u32) -> Result<(), &'static str> {
    const INVALID: &str = "Infisical-Unixsocket ist nicht als geschützte Gegenstelle bestätigt.";
    if !path.is_absolute() || path.as_os_str().len() > 107 {
        return Err(INVALID);
    }
    let components: Vec<_> = path.components().collect();
    let mut current = PathBuf::new();
    let mut after_sticky = false;
    for (index, component) in components.iter().enumerate() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err(INVALID);
        }
        current.push(component);
        let metadata = std::fs::symlink_metadata(&current).map_err(|_| INVALID)?;
        if index + 1 == components.len() {
            if after_sticky
                || !metadata.file_type().is_socket()
                || metadata.uid() != owner
                || metadata.mode() & 0o007 != 0
            {
                return Err(INVALID);
            }
        } else {
            if !metadata.is_dir() || ![0, owner].contains(&metadata.uid()) {
                return Err(INVALID);
            }
            if after_sticky && (metadata.uid() != owner || metadata.mode() & 0o077 != 0) {
                return Err(INVALID);
            }
            after_sticky = metadata.mode() & 0o022 != 0;
            if after_sticky && !(metadata.uid() == 0 && metadata.mode() & 0o1000 != 0) {
                return Err(INVALID);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::os::unix::net::UnixListener;

    #[test]
    fn only_owned_socket_inside_protected_directory_is_accepted() {
        let directory = std::env::temp_dir().join(format!("uplink-uds-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.join("api.sock");
        let listener = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let uid = nix::unistd::geteuid().as_raw();
        assert!(validate_socket(&path, uid).is_ok());
        assert!(validate_socket(&path, uid.wrapping_add(1)).is_err());
        let alias = directory.join("alias.sock");
        symlink(&path, &alias).unwrap();
        assert!(validate_socket(&alias, uid).is_err());
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(validate_socket(&path, uid).is_err());
        drop(listener);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn absent_or_tcp_address_never_creates_a_client() {
        assert!(client_builder(Path::new("http://127.0.0.1:8080"), 0).is_err());
        assert!(client_builder(Path::new("/run/absent-uplink-infisical.sock"), 0).is_err());
    }
}
