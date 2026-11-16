use std::io::ErrorKind;
use std::path::Path;

pub fn present(path: &Path) -> Result<bool, String> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err("ist keine Datei".to_string()),
        Err(err) if err.kind() == ErrorKind::NotFound => absent_or_dangling(path),
        Err(err) => Err(unchecked(&err)),
    }
}

pub fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    if !present(path)? {
        return Ok(None);
    }
    std::fs::read(path)
        .map(Some)
        .map_err(|err| unreadable(&err))
}

pub fn read_to_string(path: &Path) -> Result<Option<String>, String> {
    if !present(path)? {
        return Ok(None);
    }
    std::fs::read_to_string(path)
        .map(Some)
        .map_err(|err| unreadable(&err))
}

fn absent_or_dangling(path: &Path) -> Result<bool, String> {
    for candidate in path.ancestors() {
        if candidate.as_os_str().is_empty() {
            return Ok(false);
        }
        match std::fs::symlink_metadata(candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return match std::fs::metadata(candidate) {
                    Ok(_) => Ok(false),
                    Err(err) if err.kind() == ErrorKind::NotFound => Err(dangling(path, candidate)),
                    Err(err) => Err(unchecked(&err)),
                };
            }
            Ok(_) => return Ok(false),
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(err) => return Err(unchecked(&err)),
        }
    }
    Ok(false)
}

fn dangling(path: &Path, link: &Path) -> String {
    if link == path {
        "ist ein Link ohne Ziel".to_string()
    } else {
        format!("liegt unter dem Link ohne Ziel `{}`", link.display())
    }
}

fn unchecked(err: &std::io::Error) -> String {
    format!("konnte nicht geprüft werden: {err}")
}

fn unreadable(err: &std::io::Error) -> String {
    format!("konnte nicht gelesen werden: {err}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dl-core-optional-file-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn only_a_path_that_does_not_exist_counts_as_absent() {
        let dir = temp_dir("rules");
        let file = dir.join("datei.txt");
        std::fs::write(&file, "inhalt").expect("write");
        assert_eq!(present(&file), Ok(true));
        assert_eq!(read_to_string(&file), Ok(Some("inhalt".to_string())));
        assert_eq!(present(&dir.join("abwesend.txt")), Ok(false));
        assert_eq!(read(&dir.join("abwesend.txt")), Ok(None));
        assert_eq!(present(&dir.join("leer/abwesend.txt")), Ok(false));
        assert_eq!(present(&dir), Err("ist keine Datei".to_string()));

        let leaf = dir.join("leaf-link.txt");
        std::os::unix::fs::symlink(dir.join("ziel-weg.txt"), &leaf).expect("leaf link");
        assert_eq!(present(&leaf), Err("ist ein Link ohne Ziel".to_string()));
        assert_eq!(
            read_to_string(&leaf),
            Err("ist ein Link ohne Ziel".to_string())
        );

        let parent = dir.join("ordner-link");
        std::os::unix::fs::symlink(dir.join("ordner-weg"), &parent).expect("parent link");
        assert_eq!(
            present(&parent.join("unter/datei.txt")),
            Err(format!(
                "liegt unter dem Link ohne Ziel `{}`",
                parent.display()
            ))
        );

        let resolved = dir.join("ordner-da");
        std::fs::create_dir_all(dir.join("echt")).expect("real dir");
        std::os::unix::fs::symlink(dir.join("echt"), &resolved).expect("resolved link");
        assert_eq!(present(&resolved.join("abwesend.txt")), Ok(false));
        let under_file = present(&file.join("unter-datei.txt"));
        assert!(
            matches!(&under_file, Err(reason) if reason.starts_with("konnte nicht geprüft werden: ")),
            "{under_file:?}"
        );

        use std::os::unix::fs::PermissionsExt;
        let locked_file = dir.join("gesperrt.txt");
        std::fs::write(&locked_file, "inhalt").expect("locked file");
        std::fs::set_permissions(&locked_file, std::fs::Permissions::from_mode(0o000))
            .expect("lock file");
        let locked_dir = dir.join("gesperrt");
        std::fs::create_dir_all(&locked_dir).expect("locked dir");
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o000))
            .expect("lock dir");
        let unreadable = read(&locked_file);
        let unchecked = present(&locked_dir.join("datei.txt"));
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o700))
            .expect("unlock dir");
        assert!(
            matches!(&unreadable, Err(reason) if reason.starts_with("konnte nicht gelesen werden: ")),
            "{unreadable:?}"
        );
        assert!(
            matches!(&unchecked, Err(reason) if reason.starts_with("konnte nicht geprüft werden: ")),
            "{unchecked:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
