//! Revisionsgebundene Laufzeitdateien liegen direkt neben dem Releasebinary.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn release_root() -> Result<PathBuf> {
    let executable = std::env::current_exe().context("Pfad des laufenden Bot-Binarys fehlt")?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .context("Releasewurzel des laufenden Bot-Binarys fehlt")
}

fn validate_at(root: &Path) -> Result<()> {
    dl_community::team_applications::validate_runtime_texts(root).map_err(anyhow::Error::msg)?;
    dl_community::concierge::validate_pate_leitfaden(root).map_err(anyhow::Error::msg)?;
    Ok(())
}

pub fn validated_release_root() -> Result<PathBuf> {
    let root = release_root()?;
    validate_at(&root)?;
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fehlende_unlesbare_und_ungueltige_releaseassets_ergeben_fehler() {
        let root = tempfile::tempdir().expect("Isolierte Releaseassetprüfung");
        let assets = root.path().join("assets");
        std::fs::create_dir(&assets).expect("Assetverzeichnis");
        assert!(validate_at(root.path()).is_err());
        let team = assets.join("team_application_texts.toml");
        std::fs::create_dir(&team).expect("Unlesbare Datei als Verzeichnis");
        assert!(validate_at(root.path()).is_err());
        std::fs::remove_dir(&team).expect("Testverzeichnis entfernen");
        std::fs::write(&team, "[panel]\ntitle = [").expect("Ungültige Textdatei");
        assert!(validate_at(root.path()).is_err());
        std::fs::write(
            &team,
            include_str!("../../../../assets/team_application_texts.toml"),
        )
        .expect("Revisionsgebundene Teamtexte");
        assert!(validate_at(root.path()).is_err());
        let pate = assets.join("paten_leitfaden.toml");
        std::fs::create_dir(&pate).expect("Unlesbarer Leitfaden");
        assert!(validate_at(root.path()).is_err());
        std::fs::remove_dir(&pate).expect("Testverzeichnis entfernen");
        std::fs::write(&pate, "[leitfaden]\ntitel = [").expect("Ungültiger Leitfaden");
        assert!(validate_at(root.path()).is_err());
        std::fs::write(
            &pate,
            include_str!("../../../../assets/paten_leitfaden.toml"),
        )
        .expect("Revisionsgebundener Leitfaden");
        validate_at(root.path()).expect("Vollständige Releaseassets");
    }
}
