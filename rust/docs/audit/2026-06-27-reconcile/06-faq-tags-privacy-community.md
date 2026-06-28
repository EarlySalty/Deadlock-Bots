# Reconcile Audit 06 - FAQ/Tags/Privacy/Community

Stand: 2026-06-27  
Worker: Parallel Worker 6  
Scope: FAQ Chat, Server-FAQ, Tags, Privacy, Retention, Clips, Leave-Survey, Invites, Rules, Feedback-Hub

## Kurzfazit

- Python bleibt Source of Truth; Rust ist inaktiv und wurde nur gelesen.
- Keine High-Gaps im geprüften Scope.
- Gezählt: 13 Medium-Gaps, 15 Low-Gaps.
- Die alten High-/kritischen Punkte zu Ticket-Auto-Antwort mit Tool-Use + `diagnose_guard` Fail-Closed sind in Rust inzwischen umgesetzt.
- Ebenfalls nachgezogen: Retention Opt-out/Opt-in und Mod-Tag-Slash-Commands aus `4aa62d2`.

## ✅ Parity / erledigte alte Befunde

- FAQ Ticket-Auto-Antwort: Rust nutzt inzwischen einen Tool-Use-Loop mit Diagnose-/Log-Tools und Guard-Check. Python: `cogs/faq_chat.py:717`; Rust: `rust/crates/dl-community/src/faq.rs:881`, `rust/crates/dl-community/src/faq.rs:948`, Tests `rust/crates/dl-community/src/faq.rs:1510`.
- Ticket Guard Fail-Closed: Guard-Fehler führen in Rust zu keiner Antwort. Rust-Test: `rust/crates/dl-community/src/faq.rs:1510`.
- Retention Opt-out/Opt-in: Rust bietet Slash-Commands und Persistenz. Python: `cogs/user_retention.py:731`; Rust: `rust/crates/dl-community/src/retention.rs:229`, `rust/crates/dl-community/src/retention.rs:567`, Tests `rust/crates/dl-community/src/retention.rs:773`.
- Mod-Tag-Slash-Commands: Rust hat `/mod-tag set/remove/list`. Python: `cogs/tags/mod_commands.py:111`; Rust: `rust/crates/dl-community/src/tags_ui.rs:333`, Tests `rust/crates/dl-community/src/tags_ui.rs:472`.
- Feedback-Hub `!fhub`: Rust hat inzwischen einen Message-Listener und Panel-Post. Python: `cogs/feedback_hub.py:184`; Rust: `rust/crates/dl-community/src/feedback_hub.rs:212`, `rust/bin/dl-bot/src/main.rs:632`.
- Privacy-Core Export/Delete/Opt-Out ist weitgehend portiert. Python: `cogs/privacy_core.py:213`; Rust: `rust/crates/dl-community/src/privacy.rs:108`, `rust/crates/dl-community/src/privacy.rs:143`, `rust/crates/dl-community/src/privacy.rs:363`.

## 🔵 Deliberate / bewusst abweichend

- FAQ Ticket-Trigger: Rust verzichtet bewusst auf Python-Pending-Timeout und Name-Prefix-Heuristik; Modulkommentar dokumentiert die Annäherung. Python: `cogs/faq_chat.py:626`; Rust: `rust/crates/dl-community/src/faq.rs:12`, `rust/crates/dl-community/src/faq.rs:1003`.
- Meine-Tags UI: Python speichert erst nach `tags:save`; Rust persistiert bewusst sofort und ersetzt die Owner-Prüfung durch ephemere Interaktionen. Python: `cogs/tags/interface.py:52`, `cogs/tags/interface.py:173`; Rust: `rust/crates/dl-community/src/tags_ui.rs:4`, `rust/crates/dl-community/src/tags_ui.rs:154`.
- Privacy Runtime-Clear: Python leert Runtime-State beim Löschen; Rust kommentiert bewusst, dass Schreibpfade über Opt-Out gates laufen. Python: `cogs/privacy_controls.py:132`; Rust: `rust/crates/dl-community/src/privacy_ui.rs:3`.
- Rules Panel Publish: Python editiert eine feste Panel-Message, Rust postet bewusst frisch. Python: `cogs/rules_channel.py:218`; Rust: `rust/crates/dl-community/src/onboarding.rs:245`.

## GAPs

| ID | Severity | Typ | Python-Ref | Rust-Ref | Auswirkung | Aufwand |
| --- | --- | --- | --- | --- | --- | --- |
| FAQ-01 | Medium | Data loss / Audit | `cogs/faq_chat.py:148`, `cogs/faq_chat.py:859` | `rust/crates/dl-community/src/faq.rs:535` | `server_faq_logs` wird in Rust nicht angelegt/befüllt; FAQ-Chat-Auditdaten fehlen. | M |
| FAQ-02 | Medium | Missing feature | `cogs/server_faq.py:390`, `cogs/server_faq.py:438` | `rust/crates/dl-community/src/faq.rs:1174` | Python `/faq` erstellt Server-FAQ-Threads und `/faqclose` schließt sie; Rust hat nur FAQ-Chat/Panel-Kommandos. | M |
| FAQ-03 | Low | Behavioral | `cogs/faq_chat.py:859` | `rust/crates/dl-community/src/faq.rs:986` | Python loggt QA als Embed mit Frage/Antwort; Rust postet nur Plaintext. | S |
| FAQ-04 | Low | Behavioral | `cogs/faq_chat.py:826`, `cogs/server_faq.py:246` | `rust/crates/dl-community/src/faq.rs:150`, `rust/crates/dl-community/src/faq.rs:860` | Patchnote-Kontext wird in Rust nicht in FAQ-Antworten eingebunden. | S |
| TAG-01 | Medium | Audit gap | `cogs/tags/mod_commands.py:49`, `cogs/tags/mod_commands.py:148` | `rust/crates/dl-community/src/tags_ui.rs:233`, `rust/crates/dl-community/src/tags_ui.rs:269` | `/mod-tag set/remove` schreibt in Rust keinen Moderations-Log. | S |
| TAG-02 | Medium | Permission regression | `cogs/tags/mod_commands.py:31` | `rust/crates/dl-community/src/tags_ui.rs:190`, `rust/crates/dl-community/src/tags_ui.rs:333` | Python erlaubt Manage-Messages oder Mod-Rolle; Rust nutzt nur Default-Permission Manage-Messages. | S |
| TAG-03 | Medium | UX regression | `cogs/tags/mod_commands.py:200` | `rust/crates/dl-community/src/tags_ui.rs:288` | `/mod-tag list` zeigt in Rust die User-ID statt Displayname im Titel. | S |
| INV-01 | Medium | Missing feature | `cogs/website_invite_cog.py:253`, `cogs/website_invite_cog.py:316`, `cogs/website_invite_cog.py:375` | `rust/crates/dl-community/src/invites.rs:221` | `/website-invite`, `/website-invite-recreate` und `/join-quellen` fehlen in Rust. | M |
| INV-02 | Medium | Config regression | `cogs/website_invite_cog.py:57` | `rust/crates/dl-community/src/invites.rs:18`, `rust/bin/dl-bot/src/main.rs:543` | `WEBSITE_INVITE_CHANNEL_ID` wird in Rust nicht gelesen; Zielkanal ist fest verdrahtet. | S |
| INV-03 | Low | Behavioral | `cogs/website_invite_cog.py:179` | `rust/crates/dl-community/src/invites.rs:224`, `rust/crates/dl-community/src/invites.rs:237` | Python vergleicht Invite-Codes exakt; Rust normalisiert auf lowercase und kann theoretisch falsch klassifizieren. | S |
| INV-04 | Low | Behavioral | `cogs/website_invite_cog.py:205` | `rust/crates/dl-community/src/invites.rs:148` | Python backfillt Landing-Eintritte nach Main-Invite; Rust klassifiziert nur ohne Backfill. | S |
| PRIV-01 | Medium | Safety regression | `cogs/privacy_controls.py:21`, `cogs/privacy_controls.py:72` | `rust/crates/dl-community/src/privacy_ui.rs:41`, `rust/crates/dl-community/src/privacy_ui.rs:75` | Python nutzt zweistufige Löschbestätigung; Rust löscht nach einem Danger-Button. | S |
| PRIV-02 | Medium | Transparency regression | `cogs/privacy_controls.py:151` | `rust/crates/dl-community/src/privacy_ui.rs:75` | Python zeigt detaillierte Löschzusammenfassung inkl. Kategorien/Steam-IDs; Rust zeigt stark verkürzte Zusammenfassung. | S |
| PRIV-03 | Low | Access control | `cogs/privacy_controls.py:27` | `rust/crates/dl-community/src/privacy_ui.rs:75` | Python prüft View-Owner; Rust-Buttons haben keine explizite User-ID-Prüfung. | S |
| PRIV-04 | Low | UX regression | `cogs/privacy_controls.py:49` | `rust/crates/dl-community/src/privacy_ui.rs:92` | Export-Dateiname und Hinweistext weichen ab; Python nutzt userbezogenen Dateinamen. | S |
| RET-01 | Medium | Missing admin ops | `cogs/user_retention.py:777`, `cogs/user_retention.py:855`, `cogs/user_retention.py:915`, `cogs/leave_survey.py:682` | `rust/crates/dl-community/src/retention.rs:567`, `rust/crates/dl-community/src/leave_survey.rs:479` | Discord-Admin-Kommandos für Preview/Test/Test-DM/LeaveSurvey-Test fehlen; Status/Recent sind nur teilweise durch Dashboard-APIs ersetzt. | M |
| LS-01 | Low | UX regression | `cogs/leave_survey.py:211` | `rust/crates/dl-community/src/leave_survey.rs:249` | Select-Placeholder weicht ab. | S |
| LS-02 | Low | Audit/UX regression | `cogs/leave_survey.py:530`, `cogs/leave_survey.py:565` | `rust/crates/dl-community/src/leave_survey.rs:267`, `rust/crates/dl-community/src/leave_survey.rs:467` | Trigger-/Response-Logs sind in Python Embeds, in Rust nur Plaintext. | S |
| CLIP-01 | Medium | UX / requirement | `cogs/clip_submission.py:315` | `rust/crates/dl-community/src/clips.rs:545` | Python bestätigt die 1080p-Anforderung; Rust-Bestätigung erwähnt sie nicht. | S |
| CLIP-02 | Medium | Missing admin ops | `cogs/clip_submission.py:723` | `rust/crates/dl-community/src/clips.rs:558` | `/clips_repost` fehlt; Admins können das Clip-Interface nicht manuell neu posten. | S |
| CLIP-03 | Low | Missing admin ops | `cogs/clip_submission.py:749` | `rust/crates/dl-community/src/clips.rs:558` | `/clips winner_draw` fehlt. | S |
| CLIP-04 | Low | Validation bug | `cogs/clip_submission.py:39` | `rust/crates/dl-community/src/clips.rs:74` | Rust akzeptiert sehr kurze Links wie `http://x`; Python validiert per URL-Regex. | S |
| CLIP-05 | Low | Resilience | `cogs/clip_submission.py:381`, `cogs/clip_submission.py:495` | `rust/crates/dl-community/src/clips.rs:388` | Python findet bestehende Interface-Messages per Channel-History-Fallback; Rust nutzt nur `persistent_views`. | S |
| CLIP-06 | Low | Multi-guild behavior | `cogs/clip_submission.py:24`, `cogs/clip_submission.py:572` | `rust/crates/dl-community/src/clips.rs:22`, `rust/crates/dl-community/src/clips.rs:565` | Python kann bei `GUILD_ID=None` alle Guilds bedienen; Rust ist auf eine Guild fest verdrahtet. | S |
| FB-01 | Low | UX regression | `cogs/feedback_hub.py:147`, `cogs/feedback_hub.py:154` | `rust/crates/dl-community/src/feedback_hub.rs:108` | Feedback-DM an Admin ist in Rust Plaintext ohne Embed, Footer, Timestamp und Source-Deeplink. | S |
| FB-02 | Low | Permission/UX regression | `cogs/feedback_hub.py:184`, `cogs/feedback_hub.py:259` | `rust/crates/dl-community/src/feedback_hub.rs:216`, `rust/crates/dl-community/src/feedback_hub.rs:224` | Python erlaubt Manage-Guild und gibt Permission-Fehler aus; Rust ist admin-only und ignoriert Non-Admins still. | S |
| RULE-01 | Medium | Missing workflow | `cogs/rules_channel.py:254`, `cogs/rules_channel.py:263` | `rust/crates/dl-discord/src/gateway.rs:160`, `rust/crates/dl-community/src/onboarding.rs:300` | Auto-Start nach Discord Member Screening fehlt in Rust. | M |
| RULE-02 | Low | Resilience | `cogs/rules_channel.py:49` | `rust/bin/dl-bot/src/onboardglue.rs:134` | Python fällt bei Private-Thread-Fehlern auf Public Thread zurück; Rust versucht nur Private Thread. | S |

## Verifikation

- `cargo test -p dl-community --no-run`
- `cargo test -p dl-community`

Ergebnis: `dl-community` kompiliert und 56 Tests laufen erfolgreich durch.

## Offene Rest-Risiken

- Kein Laufzeit-Test gegen Discord/API; Bewertung basiert auf statischem Codevergleich plus Rust-Testlauf.
- Einige Command-Lücken können bewusst durch Dashboard-Endpunkte ersetzt worden sein, sind aber im Discord-Verhalten gegenüber Python nicht vollständig äquivalent.
