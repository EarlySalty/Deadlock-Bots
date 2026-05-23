# Admin Commands – Deadlock Bot

Prefix: `!` · Slash: `/` · Hybrid: `!` oder `/`

Legende: 🔴 Owner only · 🟠 Administrator · 🟢 Alle User

---

## ⚙️ Master Bot Control (`bot_core`)

| Command | Typ | Zugang | Beschreibung |
|---|---|---|---|
| `!m status` / `!master status` | Prefix | 🔴 | Bot-Status, geladene Cogs, Guilds, User |
| `!m reload <cog>` | Prefix | 🔴 | Einzelnen Cog neu laden |
| `!m reloadall` / `!m rla` | Prefix | 🔴 | Alle Cogs neu laden (auto-discovery) |
| `!m reloadsteam` / `!m rllm` | Prefix | 🔴 | Alle Steam-Cogs neu laden |
| `!m discover` / `!m disc` | Prefix | 🔴 | Neue Cogs entdecken (ohne laden) |
| `!m unload <pattern>` / `!m ul` | Prefix | 🔴 | Cogs nach Pattern entladen |
| `!m unloadtree <prefix>` / `!m ult` | Prefix | 🔴 | Alle Cogs unter einem Ordner entladen |
| `!m restart` / `!m reboot` | Prefix | 🔴 | Bot sauber neu starten |
| `!m shutdown` / `!m stop` | Prefix | 🔴 | Bot herunterfahren |

---

## 🎮 Steam Bridge

| Command | Typ | Zugang | Beschreibung |
|---|---|---|---|
| `!steam_login` | Prefix | 🟠 | Steam-Bot einloggen |
| `!steam_guard [code]` / `!sg [code]` | Prefix | 🟠 | Steam Guard Code eingeben |
| `!steam_logout` | Prefix | 🟠 | Steam-Bot ausloggen |
| `!steam_status` | Prefix | 🟠 | Steam-Bot Status anzeigen (Session, GC-Verbindung) |
| `!steam_token` | Prefix | 🟠 | Aktuellen Steam Web-Token anzeigen |
| `!steam_token_clear` | Prefix | 🟠 | Gecachten Steam-Token löschen (erzwingt Refresh) |
| `!steam_token_refresh` | Prefix | 🟠 | Steam-Token manuell refreshen |
| `!sync_steam_friends` | Prefix | 🟠 | Steam-Freundesliste mit DB synchronisieren |

---

## 🎮 Beta Invite (Deadlock Playtest)

| Command | Typ | Zugang | Beschreibung |
|---|---|---|---|
| `/betainvite` | Slash | 🟢 | Deadlock Beta-Einladung anfordern |
| `/publish_betainvite_panel` | Slash | 🟠 | Beta-Invite Panel in Kanal posten |
| `/betainvite_stats` | Slash | 🟠 | Statistiken zu Beta-Einladungen anzeigen |

---

## ✅ Steam Verified Role

| Command | Typ | Zugang | Beschreibung |
|---|---|---|---|
| `!verifyrole_run` | Prefix | 🟠 | Manueller Lauf der Rollen-Zuweisung (loggt Zuweisungen) |
| `!verifyrole_diag` | Prefix | 🟠 | Diagnose: prüft IDs, Rechte, DB & Rollenhierarchie |

---

## 🎮 Deadlock Team Balancer

| Command | Typ | Zugang | Beschreibung |
|---|---|---|---|
| `!balance auto` / `!bal auto` | Prefix | 🟢 | Team-Balance Vorschau (keine Channels) |
| `!balance start` | Prefix | 🟢 | Channels erstellen & Match starten |
| `!balance status` | Prefix | 🟢 | Spieler-Rank-Status anzeigen |
| `!balance matches` | Prefix | 🟢 | Aktive Matches anzeigen |
| `!balance end` | Prefix | 🟢 | Match beenden & Channels aufräumen |
| `!balance cleanup` | Prefix | 🟢 | Alte Matches löschen |
| `!balance manual` | Prefix | 🟢 | Manuelle Spielerauswahl für Balance |

---

## 🎙️ Deadlock Voice Status

| Command | Typ | Zugang | Beschreibung |
|---|---|---|---|
| `!dlvs trace` | Prefix | 🟠 (manage_guild) | Trace-Logging für Voice-Channels an/ausschalten |
| `!dlvs snapshot` | Prefix | 🟠 (manage_guild) | Letzten Beobachtungs-Snapshot eines Voice-Channels anzeigen |

---

## 🔢 Rank Voice Manager

| Command | Typ | Zugang | Beschreibung |
|---|---|---|---|
| `!rrang status` | Prefix | 🟠 (manage_guild) | System-Status anzeigen |
| `!rrang info` | Prefix | 🟠 (manage_guild) | User-Rank-Info |
| `!rrang debug` | Prefix | 🟠 (manage_guild) | User-Rollen debuggen |
| `!rrang anker` | Prefix | 🟠 (manage_guild) | Channel-Anker anzeigen |
| `!rrang toggle` | Prefix | 🟠 (manage_guild) | System für aktuellen VC an/ausschalten |
| `!rrang vcstatus` | Prefix | 🟠 (manage_guild) | Aktueller VC-Status |
| `!rrang rollen` | Prefix | 🟠 (manage_guild) | Rang-Rollen auflisten |
| `!rrang kanäle` | Prefix | 🟠 (manage_guild) | Überwachte/ausgeschlossene Channels |
| `!rrang aktualisieren` | Prefix | 🟠 (manage_guild) | VC manuell aktualisieren |

---

## 🔐 Security Guard

| Command | Typ | Zugang | Beschreibung |
|---|---|---|---|
| `!security_diag` | Prefix | 🟠 | Aktive Spam-Guard Schwellen anzeigen |

---

## 🎙️ Voice Tracker

| Command | Typ | Zugang | Beschreibung |
|---|---|---|---|
| `!vstats [@user]` | Prefix | 🟢 | Voice-Statistiken anzeigen (Zeit, Punkte, Live-Session) |
| `!vleaderboard` / `!vlb` / `!voicetop` | Prefix | 🟢 | Voice-Leaderboard (Top 10) |
| `!vtest` | Prefix | 🟢 | Voice-System Status (DB, Sessions, Config) |
| `!voice_status` | Prefix | 🟠 | Admin-Status: Sessions, Grace Periods, DB |
| `!voice_config [setting] [value]` | Prefix | 🟠 | Voice-Konfiguration anpassen |
| `!vf1 [@user]` | Prefix | 🟠 | Test: Tag-1 Voice-Feedback DM senden |
| `!vf4 [@user]` | Prefix | 🟠 | Test: Tag-4 Voice-Feedback DM senden |

**voice_config Settings:**
```
!voice_config grace_duration <sekunden>   # Grace Period Dauer (60–600s)
!voice_config grace_role <role_id>        # Spezielle Rolle für Grace Period
!voice_config min_users <2-10>            # Min. User für Tracking
!voice_config session_timeout <sekunden>  # Session Timeout
!voice_config max_sessions <anzahl>       # Max. Sessions pro User
```

---

## 📊 User Activity Analyzer

| Command | Typ | Zugang | Beschreibung |
|---|---|---|---|
| `!myactivity` | Prefix | 🟢 | Eigene Aktivitäts-Übersicht |
| `!useranalysis [@user]` / `!ua` / `!analyze` | Prefix | 🟢 | Umfassende User-Analyse (Voice, Messages, Events, Co-Spieler) |
| `!memberevents [@user] [limit]` / `!mevents` | Prefix | 🟢 | Letzte Member-Events (Joins, Leaves, Bans) |
| `!messagestats [@user]` / `!msgstats` | Prefix | 🟢 | Message-Statistiken eines Users |
| `!serverstats` | Prefix | 🟢 | Server-weite Statistiken |
| `!checkping [@user]` | Prefix | 🟢 | Prüft ob ein User gepingt werden kann |
| `!smartping @user [reason]` | Prefix | 🟠 (manage_messages) | Personalisierten Ping senden |

**smartping Reasons:** `join`, `game_ready`, `friends_online`, `ranked_session`

---

## 🔄 User Retention

| Command | Typ | Zugang | Beschreibung |
|---|---|---|---|
| `/retention-optout` | Slash | 🟢 | "Wir vermissen dich"-Nachrichten deaktivieren |
| `/retention-optin` | Slash | 🟢 | "Wir vermissen dich"-Nachrichten wieder aktivieren |
| `!retention_status` | Prefix | 🟠 | Retention-System Status anzeigen |
| `!retention_preview` | Prefix | 🟠 | Vorschau der Retention-Nachricht |
| `!retention_test` | Prefix | 🟠 | Retention-Nachricht testen |
| `!retention_test_dm` | Prefix | 🟠 | Retention-DM testen |
| `!retention_feedback` | Prefix | 🟠 | Feedback-Status anzeigen |

---

## 📋 Sonstige / Utility

| Command | Typ | Zugang | Beschreibung |
|---|---|---|---|
| `!set_log_channel` | Prefix | 🟠 | Log-Kanal für Bot-Logs setzen |
| `!fhub` | Prefix | 🟢 | Feedback Hub öffnen |
| `!faq` / `/faq` / `/serverfaq` | Prefix/Slash | 🟢 | Server-FAQ anzeigen |
| `/publish_rules_panel` | Slash | 🟠 | Regelwerk-Panel in Kanal posten |
| `!lfgtest` | Prefix | 🟠 | LFG-System Test |
| `!aiob` | Prefix | 🟠 | AI Onboarding-Bot triggern |
| `!tvpanel` / `!tempvoicepanel` | Prefix | 🟢 | TempVoice Interface-Panel |
| `/clips_repost` | Slash | 🟠 | Clip-Submission Interface neu erstellen |
| `/datenschutz` | Slash | 🟢 | Datenschutz-Einstellungen |
| `/datenschutz-optin` | Slash | 🟢 | Datenschutz Opt-In |
| `/streamer` | Slash | 🟢 | Partner-Onboarding starten |
| `/nudgesend` | Slash | 🟠 | Steam-Link Voice-Nudge senden |

---

## 📝 Notizen

- **`/streamer`** bleibt absichtlich erhalten. Die externe Streamer-/Twitch-Autorisierung liegt im separaten Repo `Deadlock-Twitch-Bot`; Deadlock setzt danach nur lokal Rolle und Notify.
- **Steam Bridge** muss laufen (GC-Verbindung aktiv) bevor Build-Publishing und Beta-Invites funktionieren
