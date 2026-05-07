# 🤖 Deutsche Deadlock Community Bot

[![Security: CodeQL](https://github.com/NaniDerEchte2/Deadlock/actions/workflows/codeql.yml/badge.svg)](https://github.com/NaniDerEchte2/Deadlock/actions/workflows/codeql.yml)
[![Deep Security Scan](https://github.com/NaniDerEchte2/Deadlock/actions/workflows/security-deep-scan.yml/badge.svg)](https://github.com/NaniDerEchte2/Deadlock/actions/workflows/security-deep-scan.yml)
[![Lint & Type Check](https://github.com/NaniDerEchte2/Deadlock/actions/workflows/lint-and-typecheck.yml/badge.svg)](https://github.com/NaniDerEchte2/Deadlock/actions/workflows/lint-and-typecheck.yml)
[![Secret Scanning](https://github.com/NaniDerEchte2/Deadlock/actions/workflows/secret-scanning.yml/badge.svg)](https://github.com/NaniDerEchte2/Deadlock/actions/workflows/secret-scanning.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Discord-Bot für die Deutsche Deadlock Community mit umfangreichen Features.

## Features

### 🎮 Community Features
- **Onboarding & Regelbestätigung** - Automatischer Welcome-Flow
- **Steam-Verknüpfung** - OAuth + OpenID Integration
- **TempVoice-Lanes** - Automatische Voice-Channel-Verwaltung
- **Team Balancer** - Faire Match-Erstellung
- **Voice-Activity-Tracking** - Leaderboards & Statistiken
- **Match-Coaching** - Coaching-Request-System

### 🛠️ Build-Publishing (NEU!)
- **Automatisches Build-Spiegeln** - Kopiert Builds von Top-Spielern
- **Deutsches Branding** - Community-Namen & Credits
- **Voll autonom** - Zero-Maintenance nach Aktivierung

Siehe [docs/build-publishing/](docs/build-publishing/) für Details.

### 🎥 Content Features
- **Clip-Einreichung** - Wöchentliche Clip-Sammlung
- **Feedback Hub** - Anonymes Community-Feedback

## Setup

### Installation

```bash
# Dependencies installieren
pip install -r requirements.txt

# Bot starten
python main_bot.py
```

### Konfiguration

1. `.env` Datei erstellen (siehe `.env.example`)
2. Discord Bot Token eintragen
3. Optional: Steam API Key für erweiterte Features

### Build-Publishing aktivieren

```
!load build_publisher
```

Siehe [docs/build-publishing/START.txt](docs/build-publishing/START.txt) für Details.

## Projekt-Struktur

```
Deadlock/
├── main_bot.py              # Haupt-Bot
├── cogs/                    # Discord Cogs
│   ├── build_publisher.py  # Build-Publishing-Worker (Python)
│   ├── steam/              # Steam-Integration (Node.js Build fetching)
│   └── ...                 # Weitere Cogs
├── service/                 # Business Logic
│   ├── db.py               # Datenbank
│   └── standalone_manager.py
├── data/                    # Datenbank & Exports
├── logs/                    # Log-Dateien
└── docs/                    # Dokumentation
    ├── build-publishing/   # Build-Publishing-Docs
    └── COMMUNITY_FEATURES.md
```

## Monitoring

Das System läuft vollständig autonom. Status und Queue-Informationen findest du in den Logs:

```bash
# Master Bot Logs
tail -f logs/master_bot.log | grep build_publisher

# Steam GC Logs
tail -f logs/deadlock_gc_messages.log
```

## Dokumentation

| Bereich | Dokumentation |
|---------|---------------|
| **Build-Publishing** | [docs/build-publishing/](docs/build-publishing/) |
| **Community Features** | [docs/COMMUNITY_FEATURES.md](docs/COMMUNITY_FEATURES.md) |
| **Externe Streamer-/Twitch-Integration** | [docs/TWITCH_EXTERNAL_REPO.md](docs/TWITCH_EXTERNAL_REPO.md) |
| **Allgemein** | Siehe Code-Kommentare in `cogs/` |

## Logs

```bash
# Master Bot
tail -f logs/master_bot.log

# Steam GC Messages
tail -f logs/deadlock_gc_messages.log

# Voice Status
tail -f logs/deadlock_voice_status.log
```

## Support

### Bei Problemen

1. **Logs prüfen** (siehe oben)
2. **Datenbank-Status** direkt prüfen (z.B. mit DB Browser for SQLite)
3. **Dokumentation** in `docs/` lesen

### Build-Publishing Issues

Siehe [docs/build-publishing/AUTONOMER_BETRIEB.md](docs/build-publishing/AUTONOMER_BETRIEB.md) → Troubleshooting

## Technologie-Stack

- **Python 3.11+** - Bot-Framework
- **discord.py** - Discord-Integration
- **Node.js** - Steam-Bridge (standalone)
- **SQLite** - Datenbank
- **Protobuf** - Deadlock GC-Kommunikation

## Autonome Komponenten

Der Bot nutzt einen **Standalone Manager** für autonome Hintergrund-Prozesse:

- **Steam-Bridge** - Node.js-Prozess für Steam-Integration
  - Auto-Login mit Refresh-Token
  - Auto-Reconnect bei Disconnect
  - Auto-Recovery bei Crash

Konfiguration in `service/standalone_manager.py`

## Credits

- **Deutsche Deadlock Community** - Community-Server
- **EarlySalty** - Streamer & Community-Lead
- Build-Quellen: Sanya Sniper, Cosmetical, Piggy, Average Jonas, u.a.

## Lizenz

MIT (siehe `LICENSE`)

<!-- DDC-LINK-START -->

## Deutsche Deadlock Community

Dieser Bot ist Teil der **[Deutschen Deadlock Community](https://deutsche-deadlock-community.de/)** — Master-Bot der Deutschen Deadlock Community.

- 🌐 Website: [deutsche-deadlock-community.de](https://deutsche-deadlock-community.de/)
- 💬 Discord: [discord.gg/PhkP3WgY7w](https://discord.gg/PhkP3WgY7w)
- 🎮 [Helden-Übersicht auf Deutsch](https://deutsche-deadlock-community.de/helden/) · [Anfänger-Guide](https://deutsche-deadlock-community.de/guides/anfaenger/)

<!-- DDC-LINK-END -->
