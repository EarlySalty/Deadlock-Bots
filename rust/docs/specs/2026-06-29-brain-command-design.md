# Spec: `!brain <Frage>` — Deadlock-Brain Q&A im Discord-Bot

**Datum:** 2026-06-29 · **Branch:** `rust/brain-command-2026-06-29` · **Status:** Design freigegeben, Implementierung delegiert

## Ziel

Discord-User stellen mit `!brain <Frage>` eine freie Deadlock-Frage. Der Bot
holt aus dem **Deadlock-Brain** (separates Repo, eigene SQLite-Wissensbasis) ein
trust-gewichtetes Faktenbündel + fertigen LLM-Prompt, lässt **MiniMax** daraus
eine Antwort formulieren und postet sie im Channel.

Kein Neubau der RAG-/Trust-Logik — die existiert im Brain (`dbrain-retrieval`)
und wird über die CLI `deadlock-brain ask-context` angezapft.

## Architektur (Entscheidungen, bereits freigegeben)

- **Wissenstiefe:** volles Brain-RAG (Retrieval → trust-gewichteter Kontext →
  MiniMax-Antwort). Kein reiner MiniMax-Passthrough.
- **Anbindung:** Subprozess auf die Brain-CLI. Kein neuer Dienst, keine
  Cross-Repo-Crate-Dependency.
- **Ziel:** Rust, gebaut **und deployed** (nicht dormant).

### Fluss

```
!brain <frage>
  → Gateway matcht Prefix "brain" → BrainHandler::handle(BridgeInteraction)
  → Frage extrahieren (alles nach dem Befehlswort), trimmen
  → Guards: leer→Usage · zu lang→Hinweis · Per-User-Cooldown→Hinweis
  → Subprozess: deadlock-brain ask-context "<frage>"  (JSON auf stdout)
  → JSON parsen: intent, prompt, sources
       · intent == "out_of_domain"  → kurze Absage, KEIN MiniMax-Call (spart Kosten)
       · sonst → prompt an dl-ai MiniMax (generate_text)
  → strip_think(antwort) → leer/None → "keine Antwort"-Fallback
  → Antwort in ≤2000-Zeichen-Chunks posten
```

## Komponenten

### 1. Neues Crate `rust/crates/dl-brain`
Reine Orchestrierungs-Logik hinter Ports (ohne Subprozess/Netz/Discord
testbar). In `rust/Cargo.toml` als Workspace-Member + `dl-brain` Pfad-Dep
eintragen.

```rust
// Eingehende Frage → Aktion (reine Funktion, unit-testbar)
pub enum BrainOutcome {
    Usage,                         // leere Frage
    TooLong { len: usize },        // über Max
    Cooldown { remaining_secs: u64 },
    Answer(String),                // fertige, ggf. gechunkte Antwort
    OutOfDomain,                   // intent == out_of_domain
    BackendError,                  // Subprozess/Parse/MiniMax fehlgeschlagen
}

// Ports, die die Glue im Bot implementiert:
#[async_trait] pub trait BrainRetriever {        // Subprozess-Kapsel
    // Gibt geparstes (intent, prompt, sources) oder Err zurück
    async fn ask_context(&self, frage: &str) -> Result<BrainContext, BrainError>;
}
#[async_trait] pub trait AiAnswerer {            // MiniMax-Kapsel
    async fn answer(&self, prompt: &str) -> Option<String>;
}

pub struct BrainConfig { pub max_question_len: usize, pub cooldown_secs: u64 }
pub struct BrainContext { pub intent: String, pub prompt: String, pub sources: Vec<String> }
```

Cooldown: In-Memory pro `user_id` (Mutex<HashMap<u64, Instant>>), wie andere
Cooldowns im Bot. Antwort-Chunking: an Wortgrenzen, ≤2000 Zeichen (Discord-Limit).

### 2. Glue in `rust/bin/dl-bot/src/modglue.rs`
- `BrainRetrieverGlue`: baut `tokio::process::Command` auf `BRAIN_BIN` mit
  Args `ask-context <frage>` (+ DB-Flag/Env, s.u.), Timeout (z.B. 20s), parst
  stdout-JSON (`intent`, `prompt`, `sources`).
- `BrainAiGlue`: hält `Arc<dl_ai::MiniMaxClient>`, ruft
  `generate_text(GenerateRequest { prompt, system_prompt: None, model: None,
  max_output_tokens: Some(700), temperature: 0.25 })`, danach `strip_think`.
- `BrainHandler`: implementiert `dl_discord::InteractionHandler`; extrahiert die
  Frage aus der `BridgeInteraction`, ruft die `dl-brain`-Logik, sendet Reply(s).
  Bei langer Laufzeit Typing-Indicator/Arbeits-Hinweis senden, dann Antwort.

### 3. Registrierung in `rust/bin/dl-bot/src/main.rs`
Hinter Env-Gate `BRAIN_CMD_ENABLED` (default an, wenn `BRAIN_BIN` gesetzt):
```rust
router.on_prefix("brain", Arc::new(modglue::BrainHandler { /* glue, config */ }));
```
MiniMax-Client via vorhandenem `MiniMaxClient::from_env(env)` — kein Client da
→ Befehl meldet `BackendError` (kein Panik).

## Konfiguration (ENV)
| Var | Default | Zweck |
|-----|---------|-------|
| `BRAIN_BIN` | `…/Deadlock-Brain/rust/target/release/deadlock-brain` | Pfad zur Brain-CLI |
| `BRAIN_DB` | (Brain-Default) | Pfad zur `deadlock_brain.sqlite3`, an CLI durchreichen |
| `BRAIN_CMD_ENABLED` | `1` falls `BRAIN_BIN` existiert | Feature-Gate |
| `BRAIN_COOLDOWN_SECS` | `20` | Per-User-Cooldown |
| `BRAIN_MAX_QUESTION_LEN` | `300` | Max Fragelänge |
| `BRAIN_CHANNEL_ALLOWLIST` | leer = überall | optionale Channel-IDs (CSV) |

> **Codex verifiziert am Code:** (a) wie `ask-context` die DB wählt (globales
> `--db`-Flag vs. Env vs. Default-Pfad) und reicht `BRAIN_DB` korrekt durch;
> (b) ob `ask-context` die Frage positional erwartet (ja, laut `AskContextArgs`);
> (c) exakte `BridgeInteraction`/`BridgeReply`-Felder (Frage-Text, channel_id,
> user_id, Reply-Mechanik); (d) ob `--pretty` für menschenlesbar nötig ist
> (NEIN — Default-JSON ist maschinenlesbar, das nutzen wir).

## Deutsche Texte (Claude liefert — Codex nutzt als Konstanten, schreibt KEINE eigenen DE-Texte)
Dev-Ton, locker, keine AI-Listen-Stakkato. Platzhalter `{…}` zur Laufzeit füllen.

```
BRAIN_USAGE        = "🧠 Frag mich was zu Deadlock! Z. B. `!brain wie spiel ich Vindicta?` oder `!brain ist Lash grad stark?`"
BRAIN_COOLDOWN     = "⏳ Ganz ruhig — eine Brain-Frage alle {secs}s. Gleich gehts wieder."
BRAIN_TOO_LONG     = "Das ist ja ein halber Roman 😅 — pack deine Frage in unter {max} Zeichen."
BRAIN_WORKING      = "🧠 Moment, ich wühl kurz im Brain…"
BRAIN_BACKEND_ERR  = "🧠 Mein Hirn hakt grad — probier's in ein paar Sekunden nochmal."
BRAIN_NO_ANSWER    = "🧠 Dazu find ich grad nichts Handfestes. Frag mal konkreter — Held, Item oder Fähigkeit."
BRAIN_OUT_OF_DOMAIN= "🧠 Klingt nicht nach Deadlock — dazu hab ich keine gesicherten Infos. Frag mich was zum Spiel: Held, Item, Build oder Mechanik."
```

## Fehlerbehandlung
- Subprozess: Nicht-0-Exit / Timeout / leeres stdout / JSON-Parse-Fehler → `BackendError` (geloggt via `tracing`, User sieht `BRAIN_BACKEND_ERR`).
- MiniMax `None`/leer nach `strip_think` → `BRAIN_NO_ANSWER`.
- Kein `.unwrap()` in Produktionspfaden. Fehler über `thiserror`/`anyhow`-Idiom des Repos.

## Tests (TDD, Logik im `dl-brain`-Crate, Ports gemockt)
- leere/whitespace-Frage → `Usage`
- Frage > Max → `TooLong`
- zweiter Call innerhalb Cooldown → `Cooldown`, danach wieder erlaubt
- `intent=out_of_domain` → `OutOfDomain`, **AiAnswerer NICHT aufgerufen** (Mock zählt Calls)
- normaler Pfad → `Answer`, MiniMax mit `prompt` aufgerufen, `strip_think` angewandt
- MiniMax `None` → `BackendError`/`NoAnswer`
- Chunking: >2000 Zeichen → mehrere Chunks, jeder ≤2000, an Wortgrenzen

## Deploy + Verifikation (nach Code-Freigabe, durch Claude)
1. Brain-CLI release bauen, falls `deadlock-brain`-Binary fehlt (`cargo build --release -p deadlock-brain` im Brain-Repo).
2. `dl-bot` release bauen (`cargo build --release --bin dl-bot`).
3. ENV (`BRAIN_BIN`, ggf. `BRAIN_DB`) für den dl-bot-Dienst setzen (Infisical/EnvironmentFile, nie Secret in Klartext loggen).
4. Dienst neu starten.
5. **Beweisen:** (a) neues Binary enthält das Feature (`strings`/grep nach `on_prefix`-Symbol bzw. Brain-Konstante), (b) Live-Test im Discord: `!brain <frage>` liefert echte Antwort; sonst Journal prüfen. Nicht dem Erfolgs-Log trauen.

## Out of Scope (YAGNI)
- Konversations-Verlauf / Multi-Turn.
- Semantik-Vektor-Suche (steckt im Brain, nicht hier).
- Quellen-Footer im Detail (nur wenn trivial; sonst Folge-Ticket).
- Python-Cog (Rust ist Standard, Bot ist live in Rust).
