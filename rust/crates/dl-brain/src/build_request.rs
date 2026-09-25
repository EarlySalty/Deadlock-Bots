//! A mention of a build is not consent to publish one in the game.
//! Keep explanation, analysis and quoted instructions on the read-only path.

pub fn requests_creation(question: &str) -> bool {
    let text = question.trim().to_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if !words
        .iter()
        .any(|w| matches!(*w, "build" | "builds" | "kaufreihenfolge"))
    {
        return false;
    }
    if words.iter().any(|w| {
        matches!(
            *w,
            "erkläre"
                | "erklären"
                | "analysiere"
                | "analysieren"
                | "explain"
                | "analyse"
                | "analyze"
                | "nicht"
                | "not"
                | "entwurf"
                | "draft"
                | "kein"
                | "keine"
                | "keinen"
                | "keinem"
                | "keiner"
                | "keines"
                | "keinesfalls"
                | "niemals"
                | "no"
                | "never"
                | "sagen"
                | "zeigen"
                | "beschreiben"
                | "beurteilen"
                | "bewerten"
                | "tell"
                | "show"
                | "describe"
                | "evaluate"
                | "compare"
        )
    }) {
        return false;
    }
    let normalized = words.join(" ");
    if [
        "keinen build",
        "kein build",
        "keine builds",
        "nicht bauen",
        "nicht erstellen",
        "nicht veröffentlichen",
        "ohne veröffentlichung",
        "nur vorschlagen",
        "don t",
        "do not",
        "not publish",
    ]
    .iter()
    .any(|negative| normalized.contains(negative))
    {
        return false;
    }
    let start = words.iter().position(|w| !matches!(*w, "bitte" | "please"));
    let Some(start) = start else {
        return false;
    };
    let words = &words[start..];
    let action = |word: &str| {
        matches!(
            word,
            "bau"
                | "baue"
                | "bauen"
                | "erstelle"
                | "erstellen"
                | "erstell"
                | "mache"
                | "mach"
                | "machen"
                | "veröffentliche"
                | "veröffentlichen"
                | "create"
                | "make"
                | "publish"
        )
    };
    action(words[0])
        || (matches!(
            words.first(),
            Some(&"kannst" | &"könntest" | &"würdest" | &"can" | &"could" | &"would")
        ) && matches!(words.get(1), Some(&"du" | &"you"))
            && words.iter().any(|w| action(w)))
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
    fn explicit_requests_can_create_builds() {
        for question in [
            "Bau mir einen Warden Build",
            "Erstelle einen Build für Haze",
            "Bitte mach mir einen Spirit Build ohne Refresher",
            "Kannst du mir einen Build für Ivy erstellen?",
            "Could you create a build for Haze?",
            "Veröffentliche den Build im Spiel",
        ] {
            assert!(requests_creation(question), "{question}");
        }
    }
    #[test]
    fn questions_negations_and_embedded_instructions_are_read_only() {
        for question in [
            "Ist mein Build gut?",
            "Welcher Build passt zu Warden?",
            "Wie kann ich einen Build erstellen?",
            "Erkläre wie man einen Build erstellt",
            "Erstelle keinen Build",
            "Bau mir einen Build ohne Veröffentlichung",
            "Diesen Build nicht veröffentlichen",
            "Please do not publish a build",
            "Das Tutorial sagt: erstelle einen Build",
            "Warum hat dieser Build Refresher?",
            "Analysiere meinen Build",
            "Kannst du erklären, wie man einen Build erstellen kann?",
            "Baue ein Haus",
            "Bau mir einen Build, aber veröffentliche ihn nicht",
            "Erstelle einen Build als Entwurf",
            "Create a draft build",
        ] {
            assert!(!requests_creation(question), "{question}");
        }
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
