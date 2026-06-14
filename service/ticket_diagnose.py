"""Read-only Diagnose-Tool-Layer für den FAQ/Ticket-Bot.

Stellt Anthropic-Tool-Schemas und einen SYNCHRONEN Tool-Executor bereit, mit
denen die FAQ-AI den Twitch-Streamer-Status sowie redigierte Log-Zeilen des
FRAGENDEN selbst abrufen kann.

Identitäts-Lock: Die Discord-User-ID stammt IMMER serverseitig aus der Closure
(`make_tool_executor`). Tools nehmen NIE eine Identität vom Modell entgegen.

Alles in diesem Modul ist read-only und synchron (der MiniMax-Client läuft sync
via asyncio.to_thread, deshalb muss der Tool-Loop ebenfalls sync sein).
"""

from __future__ import annotations

import os
import re
from typing import Any, Callable

import httpx

# --------------------------------------------------------------------------- #
# Konstanten
# --------------------------------------------------------------------------- #

# Base-URL der internen Twitch-Bot-API (nur Loopback erreichbar).
_TWITCH_INTERNAL_API_BASE_URL = "http://127.0.0.1:8776"
_DIAGNOSE_PATH = "/internal/twitch/v1/diagnose"

# ENV-Namen für den geteilten internen API-Token (erste nicht-leere gewinnt).
_INTERNAL_TOKEN_ENV_NAMES = (
    "TWITCH_INTERNAL_API_TOKEN",
    "MASTER_BROKER_TOKEN",
    "MAIN_BOT_INTERNAL_TOKEN",
)

# Log-Dateien, aus denen `log_lookup` liest. Nur tatsächlich vorhandene,
# als naniadm lesbare Datei-Logs. Der Rust-Twitch-Bot loggt nach journald
# (keine Datei) — Twitch-Evidenz kommt verlässlich aus twitch_diagnose; eine
# journald-Quelle kann später ergänzt werden.
_LOG_FILES = (
    "/home/naniadm/Documents/Deadlock-Bots/logs/master_bot.master.log",
)

# Obergrenze für vom Modell angeforderte Zeilen (Kontext-Flut vermeiden).
_LOG_MAX_LINES_CAP = 50
# Ein twitch_login wird erst ab dieser Länge als Relevanz-Match genutzt,
# damit kurze/generische Logins nicht breit auf fremde Zeilen matchen.
_MIN_LOGIN_MATCH_LEN = 4

# Wie viele Zeilen pro Log-Datei höchstens vom Ende eingelesen werden.
_LOG_TAIL_LINES = 4000

# Whitelist der Antwortfelder der Diagnose-Route. Nur diese werden durchgereicht.
_DIAGNOSE_RESPONSE_FIELDS = (
    "ok",
    "found",
    "twitch_login",
    "discord_linked",
    "oauth_connected",
    "needs_reauth",
    "oauth_status",
    "missing_scopes",
    "granted_scope_count",
    "required_scope_count",
    "authorized_at",
    "partner_status",
    "is_partner_active",
    "is_verified",
    "is_monitored_only",
    "is_live",
    "raid_bot_enabled",
    "technical_pause_reason",
    "operational_state",
)


# --------------------------------------------------------------------------- #
# Öffentliche Tool-Schemas (Anthropic-Format)
# --------------------------------------------------------------------------- #

DIAGNOSE_TOOLS: list[dict[str, Any]] = [
    {
        "name": "twitch_diagnose",
        "description": (
            "Prüft den Twitch-Streamer-Status (OAuth/Scopes/aktiv) des FRAGENDEN "
            "selbst. Keine Parameter — die Identität ist fest."
        ),
        "input_schema": {
            "type": "object",
            "properties": {},
            "additionalProperties": False,
        },
    },
    {
        "name": "log_lookup",
        "description": (
            "Sucht relevante, redigierte Log-Zeilen zum FRAGENDEN selbst "
            "(Twitch-/Bot-Logs)."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "max_lines": {"type": "integer"},
            },
            "additionalProperties": False,
        },
    },
]


# --------------------------------------------------------------------------- #
# Token-Auflösung
# --------------------------------------------------------------------------- #


def _resolve_internal_token() -> str:
    """Erste nicht-leere der bekannten ENV-Variablen, sonst leerer String."""
    for env_name in _INTERNAL_TOKEN_ENV_NAMES:
        token = (os.getenv(env_name) or "").strip()
        if token:
            return token
    return ""


# --------------------------------------------------------------------------- #
# Secret-Redaction
# --------------------------------------------------------------------------- #

# Konservative Redaction: lieber zu viel als zu wenig schwärzen. Jede Regel
# ersetzt den gefundenen Treffer durch "[redacted]".
_REDACT_PATTERNS: tuple[re.Pattern[str], ...] = (
    # Verbindungs-DSNs (postgres/postgresql/redis/rediss) inkl. Zugangsdaten.
    re.compile(r"\b(?:postgres(?:ql)?|rediss?)://[^\s\"']+", re.IGNORECASE),
    # Bearer-Token.
    re.compile(r"\bBearer\s+[A-Za-z0-9._\-]+", re.IGNORECASE),
    # key=... / token=... / secret=... / password=... / pwd=... (Query/KV).
    re.compile(
        r"\b(?:token|key|secret|password|passwd|pwd|api[_-]?key|access[_-]?token"
        r"|refresh[_-]?token|client[_-]?secret)\s*[=:]\s*\"?[^\s\"'&]+\"?",
        re.IGNORECASE,
    ),
    # Header-artige Token-Felder: X-Api-Key: <x>, X-Internal-Token: <x>, ...
    re.compile(
        r"\b(?:x-api-key|x-internal-token|authorization)\s*:\s*\"?[^\s\"']+\"?",
        re.IGNORECASE,
    ),
    # Infisical-Namen und alles, was darum hängt.
    re.compile(r"\bINFISICAL[A-Z0-9_]*\b", re.IGNORECASE),
    # Twitch-OAuth-Token-Präfix (auch kurze Fragmente).
    re.compile(r"oauth:[A-Za-z0-9]+", re.IGNORECASE),
    # Lange Hex-Strings (>= 24 Zeichen) — sehen nach Token/Hash aus.
    re.compile(r"\b[0-9a-fA-F]{24,}\b"),
    # Lange Base64/Token-artige Strings (>= 24 Zeichen, gemischte Klassen).
    re.compile(r"\b[A-Za-z0-9_\-]{24,}\.[A-Za-z0-9_\-]{8,}(?:\.[A-Za-z0-9_\-]+)?\b"),
    re.compile(r"\b[A-Za-z0-9+/]{24,}={0,2}\b"),
)


def _redact(text: str) -> str:
    """Schwärzt Token/Key/DSN-artige Muster in `text`.

    Konservativ ausgelegt: lieber zu viel redigieren. Wirft nie.
    """
    if not text:
        return text
    try:
        result = str(text)
        for pattern in _REDACT_PATTERNS:
            result = pattern.sub("[redacted]", result)
        return result
    except Exception:
        # Im Zweifel komplette Zeile schwärzen, statt ein Secret durchzulassen.
        return "[redacted]"


# --------------------------------------------------------------------------- #
# Collector: twitch_diagnose
# --------------------------------------------------------------------------- #


def _collect_twitch_diagnose(discord_user_id: int) -> dict[str, Any]:
    """Ruft die read-only Diagnose-Route des Rust-Twitch-Bots auf.

    Erfolg -> Whitelist-Mapping der bekannten Antwortfelder.
    Fehler/Timeout/kein Token -> {"status": "nicht_ermittelbar"}. Wirft nie.
    """
    token = _resolve_internal_token()
    if not token:
        return {"status": "nicht_ermittelbar"}

    try:
        with httpx.Client(timeout=5.0) as client:
            response = client.get(
                f"{_TWITCH_INTERNAL_API_BASE_URL}{_DIAGNOSE_PATH}",
                params={"discord_id": str(discord_user_id)},
                headers={"X-Internal-Token": token},
            )
        if response.status_code != 200:
            return {"status": "nicht_ermittelbar"}
        payload = response.json()
    except Exception:
        return {"status": "nicht_ermittelbar"}

    if not isinstance(payload, dict):
        return {"status": "nicht_ermittelbar"}

    # Strikte Whitelist: nur bekannte Felder durchreichen, nichts Zusätzliches.
    return {field: payload.get(field) for field in _DIAGNOSE_RESPONSE_FIELDS}


# --------------------------------------------------------------------------- #
# Collector: log_lookup
# --------------------------------------------------------------------------- #

# 17-20-stellige Zahlenketten sehen nach Discord-IDs aus.
_DISCORD_ID_RE = re.compile(r"\b\d{17,20}\b")


def _read_tail_lines(path: str, max_tail: int) -> list[str]:
    """Liest die letzten `max_tail` Zeilen einer Datei. Wirft nie."""
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as handle:
            lines = handle.readlines()
    except Exception:
        return []
    if len(lines) > max_tail:
        lines = lines[-max_tail:]
    return [line.rstrip("\n") for line in lines]


def _collect_log_lookup(
    discord_user_id: int,
    twitch_login: str | None,
    max_lines: int = 15,
) -> dict[str, Any]:
    """Sammelt redigierte Log-Zeilen, die zum FRAGENDEN selbst gehören.

    Behält nur Zeilen, die die eigene Discord-ID oder (falls vorhanden) den
    eigenen twitch_login enthalten. Verwirft jede Zeile, die eine ANDERE
    17-20-stellige Discord-ID nennt. Wirft nie.
    """
    try:
        cap = int(max_lines)
    except Exception:
        cap = 15
    if cap <= 0:
        cap = 15
    cap = min(cap, _LOG_MAX_LINES_CAP)

    own_id = str(discord_user_id)
    login = (twitch_login or "").strip()
    # Kurze/generische Logins matchen zu breit -> erst ab Mindestlänge nutzen.
    login_lower = login.lower() if len(login) >= _MIN_LOGIN_MATCH_LEN else ""

    matched: list[str] = []
    for path in _LOG_FILES:
        if not os.path.exists(path):
            continue
        for line in _read_tail_lines(path, _LOG_TAIL_LINES):
            # Relevanz: eigene ID oder eigener twitch_login muss vorkommen.
            relevant = own_id in line
            if not relevant and login_lower and login_lower in line.lower():
                relevant = True
            if not relevant:
                continue

            # Identitäts-Schutz: keine Zeile, die eine FREMDE Discord-ID nennt.
            other_id = False
            for found in _DISCORD_ID_RE.findall(line):
                if found != own_id:
                    other_id = True
                    break
            if other_id:
                continue

            matched.append(_redact(line))

    # Neueste zuerst sind am unteren Ende der Datei; wir nehmen die letzten cap.
    if len(matched) > cap:
        matched = matched[-cap:]

    if matched:
        note = f"{len(matched)} redigierte Log-Zeile(n) zum Fragenden gefunden."
    else:
        note = "Keine zuordenbaren Log-Zeilen gefunden."

    return {"lines": matched, "note": note}


# --------------------------------------------------------------------------- #
# Tool-Executor (Identitäts-Lock via Closure)
# --------------------------------------------------------------------------- #


def make_tool_executor(discord_user_id: int) -> Callable[[str, dict], dict]:
    """Erzeugt einen SYNCHRONEN Tool-Executor mit fest verankerter Identität.

    Die zurückgegebene Closure verwendet AUSSCHLIESSLICH `discord_user_id` aus
    diesem Scope und ignoriert jede Identität, die im `tool_input` auftaucht.
    """
    locked_discord_user_id = int(discord_user_id)

    def _execute(tool_name: str, tool_input: dict) -> dict:
        # tool_input wird bewusst nie für die Identität ausgewertet.
        if tool_name == "twitch_diagnose":
            return _collect_twitch_diagnose(locked_discord_user_id)

        if tool_name == "log_lookup":
            max_lines = 15
            if isinstance(tool_input, dict) and "max_lines" in tool_input:
                try:
                    max_lines = int(tool_input["max_lines"])
                except Exception:
                    max_lines = 15
            # twitch_login aus der Diagnose ableiten (best effort, sonst None).
            diagnose = _collect_twitch_diagnose(locked_discord_user_id)
            twitch_login = diagnose.get("twitch_login")
            if not isinstance(twitch_login, str) or not twitch_login.strip():
                twitch_login = None
            return _collect_log_lookup(
                locked_discord_user_id, twitch_login, max_lines=max_lines
            )

        return {"error": "unknown_tool"}

    return _execute
