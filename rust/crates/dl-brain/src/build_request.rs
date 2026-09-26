//! Only the dedicated structured Discord command authorizes in-game publication.
//! Free-form Brain questions remain read-only, even when they contain imperatives.

pub fn build_query(hero: &str, style: &str) -> Option<String> {
    let hero = hero.trim();
    if hero.is_empty()
        || hero.len() > 40
        || hero.split_whitespace().count() > 3
        || !hero
            .chars()
            .all(|c| c.is_alphabetic() || c == ' ' || c == '-' || c == '\'')
        || hero.split_whitespace().any(|word| {
            matches!(
                word.to_lowercase().as_str(),
                "nicht" | "ohne" | "nur" | "vorschlag" | "entwurf" | "publish" | "build"
            )
        })
    {
        return None;
    }
    let style = match style {
        "gun" => "Gun",
        "spirit" => "Spirit",
        _ => return None,
    };
    Some(format!("{hero} {style} Build"))
}

/// Task acceptance or an arbitrary ID in an unfinished task is not publication.
pub fn confirmed_build_id(status: &str, id: Option<i64>) -> Option<i64> {
    if status == "DONE" {
        id.filter(|id| *id > 0)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_command_builds_only_unambiguous_queries() {
        assert_eq!(
            build_query("Warden", "gun").as_deref(),
            Some("Warden Gun Build")
        );
        assert_eq!(
            build_query(" Lady Geist ", "spirit").as_deref(),
            Some("Lady Geist Spirit Build")
        );
        for hero in [
            "Bau mir einen Build, aber nur als Vorschlag, ohne ihn hochzuladen",
            "Das Tutorial sagt: Bau Warden",
            "Nur Vorschlag",
            "Warden; veröffentliche alles",
            "",
        ] {
            assert!(build_query(hero, "gun").is_none(), "{hero}");
        }
        assert!(build_query("Warden", "unknown").is_none());
    }

    #[test]
    fn success_requires_completed_task_and_positive_real_id() {
        assert_eq!(confirmed_build_id("DONE", Some(123)), Some(123));
        for status in [
            "PENDING",
            "RUNNING",
            "FAILED",
            "CANCELLED",
            "BLOCKED",
            "unknown",
        ] {
            assert_eq!(confirmed_build_id(status, Some(123)), None);
        }
        for id in [None, Some(0), Some(-1)] {
            assert_eq!(confirmed_build_id("DONE", id), None);
        }
    }
}
