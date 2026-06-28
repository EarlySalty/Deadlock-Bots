# Reconcile Audit: Moderation (Security Guard / AI Moderator / AI Connector)

Datum: 2026-06-27  
Scope: `cogs/security_guard.py`, `cogs/ai_moderator.py`, `cogs/ai_connector.py` gegen `rust/crates/dl-moderation/src/`, `rust/crates/dl-ai/src/lib.rs` und Discord-Glue in `rust/bin/dl-bot/src/`.

## Kurz-Summary

- GAPs gesamt: **6** (**0 high / 4 medium / 2 low**)
- SecurityGuard: Die alten High-Befunde zu Staff-Schutz, Bild-Scam-Young-Burst, Bild-Multichannel und 60-min-Propose fuer bestaetigte etablierte Streuung sind im aktuellen Rust-Code **nicht mehr zutreffend**.
- AI Moderator: W1-Fixes fuer AutoDelete-Timeout, Kontext-Backfill und reichen Prompt sind portiert. Restluecken liegen bei deaktivierter Racism-Kategorie, Self-Learning/Mod-Tagging, Logging und Case-Persistenz.
- AI Connector: Tool-Use-Loop, Modell-Normalisierung und mehrfaches `<think>`-Stripping sind portiert. Das gemeinsame MiniMax-Usage-Ledger fehlt weiter.

## Verifizierte Paritaet / Erledigt

| Bereich | Status | Nachweis |
|---|---:|---|
| SecurityGuard Staff-Schutz | ✅ PARITY | Python skippt `manage_messages`/`manage_guild` (`cogs/security_guard.py:475-478`). Rust berechnet `administrator || manage_messages || manage_guild` im Gateway (`rust/crates/dl-discord/src/gateway.rs:26-28`, `:54-76`) und Guard skippt fail-closed bei unbekanntem Staff-Status sowie `author_is_staff` (`rust/crates/dl-moderation/src/guard.rs:486-500`). |
| SecurityGuard Shadow/False-Ban-Schutz | ✅ PARITY/DELIBERATE | Rust defaultet auf `enforce=false` (`guard.rs:405-418`) und postet im Shadow-Modus nur Mod-Alert ohne Aktion (`guard.rs:892-906`). |
| Fremd-Invite-Erkennung | ✅ PARITY | Rust extrahiert echte Invite-URLs (`guard.rs:121-152`), loest Invite-Guilds auf (`guard.rs:542-557`) und routet Fremd-Invite-Treffer ueber den Guard (`guard.rs:597-630`). Glue aktualisiert eigene Invites/Vanity-Allowlist (`modglue.rs:425-501`). |
| Bild-Scam im Young-Burst | ✅ PARITY | Python nutzt Text-Check plus Bild-Check und nimmt das staerkere Signal (`cogs/security_guard.py:516-540`). Rust tut dasselbe im Young-Burst (`guard.rs:635-695`). |
| Bild-Multichannel-Verdacht | ✅ PARITY | Python Pfad 2 (`cogs/security_guard.py:546-573`) ist in Rust vorhanden (`guard.rs:699-733`) mit `AI_IMAGE_CONFIDENCE=0.75` und `IMAGE_CHANNEL_THRESHOLD=2` (`guard.rs:40-43`). |
| Bild-Scam Vision-Modell | ✅ PARITY | Python defaultet auf OpenAI `gpt-5.4-nano` (`cogs/security_guard.py:95-97`). Rust `OpenAiClient` defaultet auf `gpt-5.4-nano` (`rust/crates/dl-ai/src/lib.rs:20-22`, `:189-196`) und wird fuer Guard/AI-Mod verdrahtet (`rust/bin/dl-bot/src/main.rs:191-218`, `:363-375`). |
| Etablierte gestreute Treffer / Takeover | ✅ PARITY fuer Streuung | Rust routet Takeover oder >=2 Channels auf `GuardAction::Hijack` (`guard.rs:320-334`) und fuehrt 24h-Timeout plus Hijack-DM aus (`guard.rs:918-947`). Der alte 60-min-Propose-Befund ist dadurch erledigt. |
| AI Moderator AutoDelete | ✅ PARITY | Python loescht und timeoutet 24h (`cogs/ai_moderator.py:1105-1136`, `:1503-1524`). Rust loescht und ruft `timeout_member(..., TIMEOUT_MINUTES)` (`rust/crates/dl-moderation/src/lib.rs:549-565`). |
| AI Moderator Kontext-Backfill + Prompt | ✅ PARITY | Python eskaliert `needs_context`/0.55-0.78 mit 12 Kontextzeilen und Reply-Kontext (`cogs/ai_moderator.py:710-785`, `:1171-1218`). Rust portiert Backfill, `recent_context`, `>>>`, Zeitstempel und Reply-Kontext (`lib.rs:642-735`, `modglue.rs:120-203`). |
| AI Connector Tool-Use | ✅ PARITY | Python `generate_text_with_tools`/`generate_with_tools` (`cogs/ai_connector.py:317-428`, `:498-547`) ist in Rust als `ToolTextGenerator` und Ticket-FAQ-Konsument portiert (`rust/crates/dl-ai/src/lib.rs:614-747`, `rust/crates/dl-community/src/faq.rs:900-929`). |
| AI Connector `<think>`-Stripping | ✅ PARITY | Rust entfernt mehrere case-insensitive Blöcke in Schleife (`rust/crates/dl-ai/src/lib.rs:845-867`). |

## GAPs

| Severity | Typ | Python-Ref | Rust-Ref | Auswirkung | Aufwand |
|---|---|---|---|---|---|
| 🟡 medium | divergent-logic | `cogs/ai_moderator.py:21-38`, `:503-506` | `rust/crates/dl-moderation/src/lib.rs:58-71`, `:208-230`, Test erwartet sogar Propose bei `racism` (`:971-975`) | **Racism-Kategorie ist in Python abgeschaltet, in Rust nicht.** Python klassifiziert weiter, verwirft aber jedes Ergebnis mit Kategorie `racism`. Rust laesst `racism` als erlaubte Kategorie durch `decide_action`; hohe `delete/propose`-Confidences erzeugen Mod-Vorschlaege. Das reaktiviert genau die False-Positive-Klasse, die Python abgeschaltet hat. | S |
| 🟡 medium | missing-feature | `cogs/ai_moderator.py:903-917`, `:938-953` | `rust/crates/dl-moderation/src/lib.rs:597-632`; `ModPort` ohne Tag-Methode (`:360-390`); TagService existiert separat (`rust/crates/dl-community/src/tags.rs:257-301`) | **Self-Learning/Mod-Tag bei persistent_ragebait fehlt.** Python setzt nach 4 Ragebait-Hits den 14-Tage-Mod-Tag `ragebaiter`. Rust eskaliert nur zum Vorschlag. Downstream-Systeme, die aktive Mod-Tags auswerten, sehen den User nicht als Ragebaiter. | M |
| 🟡 medium | missing-audit-log | `cogs/ai_moderator.py:1067-1103`, `:1351-1377` | `rust/crates/dl-moderation/src/lib.rs:759-770`; `post_log` nur im Port (`rust/bin/dl-bot/src/modglue.rs:97-104`) | **Proposed/ragebait_escalated Cases schreiben keinen Log-Channel-Eintrag.** Python postet fuer jeden Vorschlag ein Log-Embed plus Originaltext. Rust legt den Case an und postet Review-Buttons, aber keinen Log. Moderationsvorschlaege sind damit im Log-Channel nicht voll auditierbar. | S |
| 🟡 medium | data-loss | `cogs/ai_moderator.py:1138-1169`, `:1544-1578`, `:1233-1350` | `rust/crates/dl-moderation/src/store.rs:7-19`, `:93-109`; Draft ohne Zusatzdaten (`rust/crates/dl-moderation/src/lib.rs:738-756`) | **Attachments, `ai_raw_json` und `escalated_with_context` werden nicht persistiert.** Das Rust-Schema hat die Spalten, aber `CaseDraft`/`insert_case` fuellen sie nie. Review/Log koennen Bild-Anhaenge nicht wie Python rendern, raw AI und Kontext-Eskalation gehen fuer Review/Self-Learning verloren. | M |
| ⚪ low | port-bug | `cogs/ai_moderator.py:1682-1729` | `rust/crates/dl-moderation/src/store.rs:215-249` | **Ragebait-Fenster zaehlt ohne `guild_id`.** Python filtert Hits nach `user_id` und `guild_id`; Rust filtert nur nach `user_id`. In der aktuellen Single-Guild-Praxis kaum sichtbar, bei mehreren Guilds kann derselbe User guild-uebergreifend falsch eskalieren. | S |
| ⚪ low | missing-observability | `cogs/ai_connector.py:30-60`, `:293-300`, `:381-389` | ABSENT in `rust/crates/dl-ai/src/lib.rs` | **Gemeinsames MiniMax-Usage-Ledger fehlt.** Python schreibt MiniMax-Tokenverbrauch best-effort nach `~/Documents/.claude/minimax-usage`; Rust extrahiert Text/Tool-Calls, persistiert aber keine Usage. Das bricht Kosten-/Budget-Observability, nicht die Moderationsentscheidung selbst. | S/M |

## 🔵 DELIBERATE / Nicht als GAP gewertet

| Thema | Bewertung | Nachweis |
|---|---|---|
| Neu-Account-Kriterium | 🔵 DELIBERATE | Python `SecurityGuard._is_new_account` nutzt aktuell nur Account-Alter (`cogs/security_guard.py:607-612`). Rust nutzt `<30d Account UND <7d Server` (`guard.rs:301-307`). Das weicht von Python-live ab, ist aber durch die explizite Spec `rust/docs/specs/2026-06-21-invite-newaccount-moderation-design.md:12`, `:51` und den aktuellen Rust-Kommentar `guard.rs:13-15` als neue Rust-Policy belegt. |
| Etablierter Einzelchannel-Scam | 🔵 DELIBERATE | Python `_handle_scam_proposal` timeoutet etablierte Accounts 24h plus Hack-DM (`cogs/security_guard.py:1086-1212`). Rust routet etablierte Einzelchannel-Treffer auf `SoftWarn`, >=2 Channels/Takeover auf `Hijack` (`guard.rs:320-334`, `:852-982`). Die Spec beschreibt exakt diese Konsolidierung: 1 Channel Soft-Warn, >=2 Channels Hijack (`rust/docs/specs/2026-06-21-invite-newaccount-moderation-design.md:31-33`, `:57`, `:95`, `:101`, `:115`). Daher nicht als Gap gegen den Rust-Port gewertet, aber beim Cutover bewusst zu bestaetigen. |
| SecurityGuard `security_diag` | 🔵 low/out-of-focus | Python hat `!security_diag` (`cogs/security_guard.py:1761-1782`), Rust nicht. Das ist eine Admin-Diagnose-Coverage-Luecke aus dem alten Audit, aber nicht Teil der aktuellen Moderations-Schutzpfade. |
