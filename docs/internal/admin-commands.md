# Admin-Commands

## Zweck
Diese Datei ist die interne Sammelstelle fuer Moderations-, Admin- und Owner-Commands des Master-Bots. Grundlage sind die Legacy-Uebersicht in `docs/admin_commands.md` sowie ein Code-Scan nach explizit admin- oder owner-gebundenen Commands. Sie dient als schnelle Referenz fuer Betrieb, Incident-Handling und Moderation.

## Architektur
Es gibt drei Quellen fuer Berechtigungen:

- Legacy-Doku in `docs/admin_commands.md` dokumentiert den operativen Bestand, auch wenn nicht jeder Command in diesem Worker-Pass erneut vollstaendig im Code verifiziert wurde.
- Prefix-/Hybrid-Commands sichern sich meist ueber `@commands.has_permissions(...)`.
- Slash-Commands nutzen `@app_commands.default_permissions(administrator=True)` oder `@app_commands.checks.has_permissions(...)`.

Im gescannten `cogs/`-Baum gab es keine direkten Treffer auf `@commands.is_owner()`. Owner-Commands fuer Laden, Reload und Neustart stammen hier deshalb aus der Legacy-Doku bzw. dem Loader-/Master-Control-Bereich und sind fuer den normalen Mod-Betrieb nicht gedacht.

## Konfiguration
Relevante Rechteklassen:

- `Owner`: Bot-Besitzer bzw. Loader-/Master-Control.
- `Administrator`: volle Server-Admins.
- `Manage Guild`: Server-Setup/Voice-Struktur.
- `Manage Messages`: Moderations-Review, Smart-Pings, AI-Mod-Review.

## Admin-Workflow
Typischer Ablauf:

1. Erst passenden Command in der Tabelle finden.
2. Vor Live-Eingriffen Diagnose-Commands nutzen, z. B. `!security_diag`, `!verifyrole_diag`, `!voice_status`.
3. Erst danach mutierende Commands ausfuehren, z. B. `!voice_config`, `/publish_rules_panel`, `/faqpanel`.
4. Owner-only Loader-Commands nur fuer Deployments, Hotfixes oder defekte Cogs nutzen.

## Datenmodell
Die Commands selbst haben kein zentrales Datenmodell. Viele der hier gelisteten Befehle arbeiten aber direkt mit zentralen SQLite-Tabellen wie `kv_store`, `voice_stats`, `claimed_threads`, `ai_moderation_cases` oder `security_guard_incidents`.

## Command-Tabelle
| Command | Was er macht | Wer darf |
|---|---|---|
| `!m status`, `!master status` | Zeigt Bot-Status, geladene Cogs und Guild-Kontext. | Owner |
| `!m reload <cog>` | Laedt genau ein Cog neu. | Owner |
| `!m reloadall`, `!m rla` | Laedt alle Cogs neu. | Owner |
| `!m reloadsteam`, `!m rllm` | Laedt Steam-bezogene Cogs neu. | Owner |
| `!m discover`, `!m disc` | Erkennt neue Cogs ohne sofortiges Laden. | Owner |
| `!m unload <pattern>`, `!m ul` | Entlaedt Cogs nach Pattern. | Owner |
| `!m unloadtree <prefix>`, `!m ult` | Entlaedt ganze Cog-Unterbaeume. | Owner |
| `!m restart`, `!m reboot` | Startet den Bot sauber neu. | Owner |
| `!m shutdown`, `!m stop` | Faehrt den Bot herunter. | Owner |
| `!steam_login`, `!steam_guard`, `!steam_logout`, `!steam_status` | Bedient die Steam-Bridge manuell. | Administrator |
| `!steam_token`, `!steam_token_clear`, `!steam_token_refresh` | Diagnostik und Reset fuer Steam-Web-Token. | Administrator |
| `!sync_steam_friends` | Stoesst Freundeslisten-Sync an. | Administrator |
| `/publish_betainvite_panel` | Postet das Beta-Invite-Panel. | Administrator |
| `/betainvite_stats` | Zeigt Invite-Statistiken. | Administrator |
| `!verifyrole_run`, `!verifyrole_diag` | Steam-Verified-Rollenlauf oder Diagnose. | Administrator |
| `!dlvs trace`, `!dlvs snapshot` | Voice-Status-Trace und Snapshots. | `manage_guild` |
| `!rrang status`, `!rrang info`, `!rrang debug`, `!rrang anker`, `!rrang toggle`, `!rrang vcstatus`, `!rrang rollen`, `!rrang kanäle`, `!rrang aktualisieren` | Betrieb und Debug des Rank-Voice-Managers. | `manage_guild` |
| `!security_diag` | Zeigt aktive Security-Guard-Schwellen und Zielkanaele. | Administrator |
| `!voice_status`, `!voice_config`, `!vf1`, `!vf4` | Voice-Tracking-Diagnose, Runtime-Konfig und Test-DMs. | Administrator |
| `!smartping` | Sendet personalisierte Admin-Pings. | `manage_messages` |
| `!serverstats`, `!rawmember` | Erweiterte Server-/Member-Diagnosen. | `manage_guild` |
| `!retention_status`, `!retention_preview`, `!retention_test`, `!retention_test_dm`, `!retention_feedback` | Betrieb und Test des Retention-Systems. | Administrator |
| `!set_log_channel` | Setzt den zentralen Bot-Log-Channel. | Administrator |
| `!lfgtest`, `!lfgroute` | Test und Routing-Diagnose fuer LFG. | Administrator |
| `!leavesurvey_status`, `!leavesurvey_test`, `!leavesurvey_recent` | Diagnose/Test fuer Leave-Survey. | Administrator |
| `!tvpanel` | Postet das TempVoice-Panel neu. | `manage_guild` |
| `!fhub` | Postet/oeffnet den Feedback Hub. | `manage_guild` |
| `!aiob` | Startet AI-Onboarding-Test per DM. | Administrator |
| `/publish_rules_panel` | Postet das Regelwerk-Panel. | Administrator |
| `/faqpanel` | Postet das FAQ-Panel fuer den FAQ-Bot. | Administrator |
| `/nudgesend` | Schickt einen Steam-Link-Voice-Nudge manuell. | Administrator |
| `/clips_repost` | Repostet das Clip-Submission-Interface. | Administrator laut Legacy-Doku |

## Wartung & Troubleshooting
- Wenn ein Legacy-Command nicht mehr existiert, zuerst den Loader-Stand und die betroffene Cog-Datei pruefen.
- Bei Slash-Commands mit fehlender Sichtbarkeit meist Discord-Permissions oder noch nicht synchronisierte App-Commands pruefen.
- Owner-Commands nicht als Ersatz fuer fachliche Admin-Commands nutzen; sie umgehen nur den Cog-Lifecycle, nicht fachliche Preconditions.

## Code-Referenz
- Legacy-Quelle: `docs/admin_commands.md`
- Slash-Admin-Scan: `cogs/faq_chat.py:750`, `cogs/rules_channel.py:178`
- Prefix-Admin-Scan: `cogs/ai_connector.py:560`, `cogs/security_guard.py:1298`, `cogs/steam_verified_role.py:808`, `cogs/voice_activity_tracker.py:1593`, `cogs/user_retention.py:777`, `cogs/helper/log_bridge.py:138`
