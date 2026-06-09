"""
Bot -> Website-Brücke fuer das Coaching.

Spiegelt Coaching-Anfragen/Sessions an die Website-API
(``POST {WEBSITE_API_BASE}/coaching/platform/sync``) mit dem geteilten
``COACHING_BOT_TOKEN`` (Header ``X-Bot-Token``).

Strikt best-effort: faellt die Website aus oder ist kein Token gesetzt, wird
nur geloggt — der Discord-Coaching-Flow darf dadurch nie blockieren.
"""

from __future__ import annotations

import logging
import os
from typing import Any

import aiohttp

from service.config import settings
from service.http_client import build_resilient_connector

log = logging.getLogger(__name__)


def _base_url() -> str:
    base = getattr(settings, "website_api_base", "") or "https://deutsche-deadlock-community.de/api"
    return base.rstrip("/")


# Gleicher interner Token wie der restliche Stack (master_broker/public_stats),
# liegt bereits im Bot-Env. Kein eigenes COACHING_BOT_TOKEN noetig.
_TOKEN_ENV_NAMES = ("TWITCH_INTERNAL_API_TOKEN", "MASTER_BROKER_TOKEN")


def _token() -> str:
    for name in _TOKEN_ENV_NAMES:
        val = (os.getenv(name) or "").strip()
        if val:
            return val
    # Fallback: optionaler COACHING_BOT_TOKEN
    tok = getattr(settings, "coaching_bot_token", None)
    if tok is None:
        return ""
    return tok.get_secret_value() if hasattr(tok, "get_secret_value") else str(tok)


async def sync_coaching(payload: dict[str, Any]) -> bool:
    """Sendet einen Snapshot an die Website. True bei Erfolg, sonst False (nie raise)."""
    token = _token()
    if not token:
        log.debug("Kein interner Token gesetzt – Website-Coaching-Sync uebersprungen")
        return False

    url = f"{_base_url()}/coaching/platform/sync"
    try:
        connector = build_resilient_connector()
        timeout = aiohttp.ClientTimeout(total=10)
        async with aiohttp.ClientSession(connector=connector, timeout=timeout) as session:
            async with session.post(
                url, json=payload, headers={"X-Internal-Token": token, "X-Bot-Token": token}
            ) as resp:
                if resp.status >= 400:
                    body = (await resp.text())[:200]
                    log.warning("Website-Coaching-Sync fehlgeschlagen (%s): %s", resp.status, body)
                    return False
                return True
    except Exception as exc:  # best-effort, niemals den Bot-Flow brechen
        log.warning("Website-Coaching-Sync Fehler: %s", exc)
        return False


async def sync_coaches(coaches: list[dict]) -> bool:
    """Pusht die aktuelle Coach-Liste an die Website. True bei Erfolg, sonst False (nie raise)."""
    token = _token()
    if not token:
        log.debug("Kein interner Token gesetzt – Coach-Sync übersprungen")
        return False

    url = f"{_base_url()}/coaching/platform/coaches/sync"
    try:
        connector = build_resilient_connector()
        timeout = aiohttp.ClientTimeout(total=10)
        async with aiohttp.ClientSession(connector=connector, timeout=timeout) as session:
            async with session.post(
                url,
                json={"coaches": coaches},
                headers={"X-Internal-Token": token, "X-Bot-Token": token},
            ) as resp:
                if resp.status >= 400:
                    body = (await resp.text())[:200]
                    log.warning("Coach-Sync fehlgeschlagen (%s): %s", resp.status, body)
                    return False
                return True
    except Exception as exc:
        log.warning("Coach-Sync Fehler: %s", exc)
        return False


async def get_due_notifications() -> list[dict]:
    """Holt fällige Termin-Benachrichtigungen von der Website. Bei Fehler: [] (nie raise)."""
    token = _token()
    if not token:
        log.debug("Kein interner Token gesetzt – Notification-Poll übersprungen")
        return []

    url = f"{_base_url()}/coaching/platform/notifications/due"
    try:
        connector = build_resilient_connector()
        timeout = aiohttp.ClientTimeout(total=10)
        async with aiohttp.ClientSession(connector=connector, timeout=timeout) as session:
            async with session.get(
                url,
                headers={"X-Internal-Token": token, "X-Bot-Token": token},
            ) as resp:
                if resp.status >= 400:
                    body = (await resp.text())[:200]
                    log.warning("Notification-Poll fehlgeschlagen (%s): %s", resp.status, body)
                    return []
                data = await resp.json()
                return data.get("notifications", [])
    except Exception as exc:
        log.warning("Notification-Poll Fehler: %s", exc)
        return []


async def ack_notifications(items: list[dict]) -> bool:
    """Bestätigt verarbeitete Benachrichtigungen. True bei Erfolg, sonst False (nie raise)."""
    if not items:
        return True
    token = _token()
    if not token:
        log.debug("Kein interner Token gesetzt – Notification-Ack übersprungen")
        return False

    url = f"{_base_url()}/coaching/platform/notifications/ack"
    try:
        connector = build_resilient_connector()
        timeout = aiohttp.ClientTimeout(total=10)
        async with aiohttp.ClientSession(connector=connector, timeout=timeout) as session:
            async with session.post(
                url,
                json={"items": items},
                headers={"X-Internal-Token": token, "X-Bot-Token": token},
            ) as resp:
                if resp.status >= 400:
                    body = (await resp.text())[:200]
                    log.warning("Notification-Ack fehlgeschlagen (%s): %s", resp.status, body)
                    return False
                return True
    except Exception as exc:
        log.warning("Notification-Ack Fehler: %s", exc)
        return False
