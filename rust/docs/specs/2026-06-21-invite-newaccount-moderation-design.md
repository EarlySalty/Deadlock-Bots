# Design: Fremd-Invite-Erkennung + Neu-Account-Eskalation (Security Guard)

**Datum:** 2026-06-21
**Status:** Entwurf zur Review
**Betrifft:** Rust-Port `rust/crates/dl-moderation` (+ Glue `rust/bin/dl-bot/src/modglue.rs`, `rust/crates/dl-discord`). Python-Referenz `cogs/security_guard.py` bleibt unverändert.

## Ziel

Der Security Guard soll zwei neue/erweiterte Fähigkeiten bekommen:

1. **Fremd-Discord-Invite-Erkennung:** Nachrichten mit einem Discord-Invite, der **nicht** zu unserem Server gehört, werden wie Scam behandelt.
2. **Neu-Account-Eskalation:** Accounts, die **neu auf Discord (< 30 Tage)** UND **neu auf dem Server (< 7 Tage)** sind, werden bei einem Treffer direkt gebannt — aber **nie pauschal**, sondern nur bei tatsächlichem Scam-/Fremd-Link-Treffer.

Beide speisen in **ein** vereinheitlichtes 3-Stufen-Aktionsmodell ein.

## Nicht-Ziele (Scope-Grenzen)

- Keine Erkennung von Nicht-Invite-Discord-Links (Nachrichten-/Channel-/User-Links) — nur echte Invites.
- Kein pauschaler Ban neuer Accounts ohne Treffer.
- Keine echte „ephemeral"-Nachricht (Discord-API-Grenze ohne Interaktion) — stattdessen selbstlöschende Channel-Notiz.
- Python-Original (`cogs/`, `service/`) wird nicht geändert (Rust-Port ist Ziel).

## Aktionsmodell (3 Stufen)

**Treffer** = bestehendes Scam-Signal (Keyword+AI, Young-Burst, Bild-Multichannel) **ODER** fremder Discord-Invite.

| Stufe | Bedingung | Aktion |
|---|---|---|
| **Ban** | Account < 30d Discord **UND** < 7d Server | DM (@earlysalty-Kontakt) **zuerst** → dann Ban + Nachricht(en) löschen |
| **Soft-Warn** | Etabliert, **< 2 betroffene Channels** im Fenster (Einzel-Channel, egal ob 1 oder mehr Nachrichten) | Nachricht löschen + kurze selbstlöschende Erwähnung im Channel (kein Timeout, keine DM) |
| **Takeover/Hijack** | Etabliert mit Streuung über **≥ 2 Channels** im Fenster (= ≥ 2 Nachrichten in verschiedenen Channels) **ODER** Takeover-Muster | Nachricht(en) löschen + 24h-Timeout (reversibel) + Hijack-DM |

**Wichtige Konsolidierung:** „Takeover" und „Hijack" sind derselbe Fall (kompromittierter etablierter Account). Die in Welle 1 separat gebauten `GuardAction::Takeover` und `GuardAction::EstablishedScam` werden zu **einer** Aktion zusammengeführt (24h-Timeout reversibel + Hijack-DM), getriggert durch beide Signale.

### Reihenfolge (zwingend)

`DM → dann Ban`. Nach einem Ban kann der Bot keine DM mehr öffnen (kein gemeinsamer Server) — die DM **muss** vor dem Ban raus. Der bestehende Code wertet den DM-Block bereits vor dem Aktions-Block aus; das bleibt so. DM-Fehlschlag (geschlossene DMs) blockiert den Ban **nicht**.

## Account-Klassifikation

Vorhanden im Event: `author_created_at` (Account-Erstellung), `author_joined_at` (Server-Beitritt, `Option`).

- **neu (ban-fähig):** `now - created_at < 30d` **UND** `joined_at` vorhanden und `now - joined_at < 7d`.
  - Wenn `joined_at` fehlt (unbekannt): konservativ als **nicht** ban-fähig behandeln → fällt in etabliert/Soft-Warn (kein Ban auf unsicherer Datenbasis).
- **etabliert:** alles andere.

Neue Konstanten (idiomatisch zu den bestehenden in `guard.rs`):
- `NEW_ACCOUNT_MAX_AGE_HOURS = 720` (30d) — entspricht bereits `ACCOUNT_MAX_AGE_HOURS`.
- `NEW_MEMBER_MAX_JOIN_HOURS = 168` (7d) — **neu**.

Die bestehende `is_new_account`/`is_established_account`-Logik wird auf dieses Kriterium vereinheitlicht (Tier-Vereinheitlichung #1): Ban-Schwelle für **beide** Pfade (bestehender Scam + Fremd-Link) = „< 30d Discord UND < 7d Server".

## Streuungs-Messung (Soft-Warn vs. Takeover)

Reuse des vorhandenen Zeitfenster-Verlaufs pro User (`WINDOW_SECONDS = 3600`). Bei einem etablierten Treffer:
- distinct Channels im Fenster zählen (inkl. aktueller Nachricht).
- `>= 2` Channels → Takeover/Hijack; sonst → Soft-Warn.

Kein neuer Tracking-State — der Guard hält den Channel-übergreifenden Verlauf bereits.

## Fremd-Invite-Erkennung

### Extraktion (pur, in `dl-moderation`, testbar)
Regex über den Nachrichtentext, erkennt Invite-Codes aus:
- `discord.gg/<code>`
- `discord.com/invite/<code>`, `discordapp.com/invite/<code>`
- `ptb.`/`canary.`-Varianten
- Vanity-Form (`discord.gg/<vanity>`)

Nur Invites — **keine** `discord.com/channels/…`, `/users/…` etc.

### Auflösung „eigen vs. fremd" (Hybrid + Auto-Allowlist, im Glue/Port)
Reihenfolge pro extrahiertem Code:
1. **Allowlist-Schnellpfad:** Code in der Allowlist eigener Invites/Vanity? → eigen, ignorieren (kein API-Call).
2. **API-Auflösung:** sonst `GET /invites/{code}` → liefert `guild_id`.
   - `guild_id == OUR_GUILD_ID` → eigen, ignorieren.
   - `guild_id != OUR_GUILD_ID` → **fremd** → Treffer.
   - **nicht auflösbar** (404/expired/Fehler) → wie **fremd** behandeln (Treffer). Begründung: nicht-auflösbare Invites sind eher verdächtiger.
3. Ergebnis pro Code **cachen** (TTL), um wiederholte Calls zu vermeiden.

**Auto-Allowlist:** beim Start des Bots die eigenen Invites + Vanity der Guild via API holen und in die Allowlist schreiben (periodisch erneuern). Zusätzlich optionaler **Config-Fallback** (fest hinterlegte eigene Invite-URL/Vanity), falls die Auto-Allowlist mal nicht greift.

### Schnittstellen-Trennung (deep modules)
- `dl-moderation` (pur, testbar): Invite-Code-Extraktion, Klassifikation (neu/etabliert), Streuungs-Logik, Aktions-Entscheidung. Kennt nur eine Port-Methode `resolve_invite_guild(code) -> Option<u64>` (oder `is_own_invite(code) -> bool`).
- Glue (`modglue.rs`/`dl-discord`): implementiert Invite-Auflösung gegen die Discord-API inkl. Allowlist + Cache. Hält keine Entscheidungslogik.

## User-sichtbare Texte (von Claude, final)

**Ban-DM (neuer Account, vor Ban):**
> Du wurdest auf der Deutschen Deadlock Community gebannt, weil dein Account ein Scam-/Fremdlink-Muster ausgelöst hat. Wenn du denkst, das ist ein Fehler: Schick @earlysalty eine Freundschaftsanfrage **und** eine kurze Nachricht — dann schauen wir uns das an.

**Soft-Warn (selbstlöschende Channel-Notiz, ~12s):**
> `<@user>` — fremder Discord-Invite entfernt. Bitte keine fremden Server-Einladungen posten.

**Hijack-DM (Takeover/etabliert mit Streuung):** der in Welle 1 für `EstablishedScam` formulierte Text (Account evtl. gekapert, 24h, melde dich beim Mod-Team) wird zur einheitlichen Hijack-DM; ggf. minimal an die Takeover-Formulierung angeglichen.

`@earlysalty` als Kontakt-Handle wird konfigurierbar gehalten (Config), nicht hart im Code.

## Mod-Channel

Bei **Ban** und **Takeover/Hijack** weiterhin ein Mod-Embed posten (bestehender Pfad), mit Aktion, Account-Alter, Server-Beitritt, Streuung (Channel-Anzahl) und Auslöser (Fremd-Invite vs. Scam). Soft-Warn braucht **kein** Mod-Embed (zu geringfügig) — optional nur Log.

## Konfiguration

- `OUR_GUILD_ID` (vorhanden/aus Config).
- `INVITE_ALLOWLIST_FALLBACK` (optional, eigene Invite-Codes/Vanity).
- `ESCALATION_CONTACT_HANDLE` (Default `@earlysalty`).
- Konstanten in `guard.rs` (`NEW_MEMBER_MAX_JOIN_HOURS = 168` etc.).

## Tests (DoD)

`cargo check --workspace` + `cargo test -p dl-moderation` grün. Neue Unit-Tests (pur, ohne Netzwerk):
- Invite-Code-Extraktion: erkennt alle Invite-Formen, ignoriert Nicht-Invite-discord.com-Links.
- Klassifikation: neu (< 30d & < 7d), etabliert, `joined_at`=None → nicht ban-fähig.
- Streuung: 1 Channel → Soft-Warn, ≥2 Channels → Takeover/Hijack.
- Aktions-Routing: (Treffer × Standing × Streuung) → korrekte `GuardAction`.
- Konsolidierung: Takeover-Muster und etabliert-≥2-Channel ergeben dieselbe Aktion.
- Reihenfolge: DM wird vor Ban ausgelöst (über Port-Aufruf-Reihenfolge prüfbar).

Auflösungs-/Cache-Logik im Glue: gegen eine Fake-Port-Implementierung getestet (eigen/fremd/nicht-auflösbar).

## Offene Annahmen

- `OUR_GUILD_ID` ist im Bot bereits bekannt (single-guild Bot).
- Discord-API erlaubt dem Bot `GET /invites/{code}` (öffentlich, kein Sonderrecht nötig) und das Lesen eigener Guild-Invites (Permission `Manage Guild`, für Auto-Allowlist).
