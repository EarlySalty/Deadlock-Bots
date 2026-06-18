"""Discord-View für den Conversation-Scam-Guard „Rückgängig"-Button.

Der Twitch-Bot (Rust) postet über den Master-Broker ein Embed mit einem
`view_spec` vom Typ `scam_revoke`. Dieser baut hier einen persistenten Button.
Ein Klick nimmt die Scam-Entscheidung zurück, indem die interne Twitch-API
aufgerufen wird:

    POST http://127.0.0.1:8776/internal/twitch/v1/scam-guard/revoke
    Body: {"verdictId": <int>}
    Header: X-Internal-Token: <token>

Der Server entbannt (sofern gebannt) und markiert das Verdict als `overturned`,
was das Self-Learning des Guards mit einem Fehlalarm füttert.

Persistenz-Hinweis: Die View wird zur Laufzeit via `bot.add_view` registriert
und bleibt nur bis zum nächsten Bot-Neustart aktiv. Es gibt bewusst keinen
Rehydrate-Mechanismus — dauerhafte Rücknahme-Pfade sind `!unban` im Chat und
das Dashboard.
"""

from __future__ import annotations

import logging
import os
from typing import Any

import aiohttp
import discord

logger = logging.getLogger(__name__)

# --------------------------------------------------------------------------- #
# Konstanten
# --------------------------------------------------------------------------- #

_TWITCH_INTERNAL_API_BASE_URL = "http://127.0.0.1:8776"
_REVOKE_PATH = "/internal/twitch/v1/scam-guard/revoke"

# ENV-Namen für den geteilten internen API-Token (erste nicht-leere gewinnt) —
# identisch zur Auflösung in service/ticket_diagnose.py.
_INTERNAL_TOKEN_ENV_NAMES = (
    "TWITCH_INTERNAL_API_TOKEN",
    "MASTER_BROKER_TOKEN",
    "MAIN_BOT_INTERNAL_TOKEN",
)

# Deutsche UI-Texte (bewusst von Hand gepflegt, keine Maschinen-Generierung).
_BUTTON_LABEL_ACTIVE = "Rückgängig"
_BUTTON_LABEL_REVOKED = "Zurückgenommen"
_FOLLOWUP_SUCCESS = "Entscheidung zurückgenommen."
_FOLLOWUP_FAILURE = "Rücknahme fehlgeschlagen – bitte später erneut versuchen."


# --------------------------------------------------------------------------- #
# Token-Auflösung + interner API-Aufruf
# --------------------------------------------------------------------------- #


def _resolve_internal_token() -> str:
    """Erste nicht-leere der bekannten ENV-Variablen, sonst leerer String."""
    for env_name in _INTERNAL_TOKEN_ENV_NAMES:
        token = (os.getenv(env_name) or "").strip()
        if token:
            return token
    return ""


def revoke_url() -> str:
    """Voll-qualifizierte URL der internen Revoke-Route."""
    return f"{_TWITCH_INTERNAL_API_BASE_URL}{_REVOKE_PATH}"


async def post_revoke(
    verdict_id: int,
    *,
    session: aiohttp.ClientSession | None = None,
) -> bool:
    """POSTet die Rücknahme an die interne Twitch-API.

    Gibt True bei 2xx-Antwort zurück, sonst False. Eine fehlende Token-Config
    oder ein Netzwerkfehler führt zu False (kein Raise) — der Aufrufer meldet
    dem Klickenden einen sauberen Fehlerhinweis.
    """
    token = _resolve_internal_token()
    if not token:
        logger.error("scam-revoke: kein interner API-Token konfiguriert")
        return False

    payload = {"verdictId": int(verdict_id)}
    headers = {"X-Internal-Token": token, "Content-Type": "application/json"}

    owns_session = session is None
    if session is None:
        session = aiohttp.ClientSession()
    try:
        async with session.post(revoke_url(), json=payload, headers=headers) as response:
            ok = 200 <= response.status < 300
            if not ok:
                logger.warning(
                    "scam-revoke: Revoke fehlgeschlagen (verdict=%s status=%s)",
                    int(verdict_id),
                    response.status,
                )
            return ok
    except Exception:
        logger.exception("scam-revoke: HTTP-Aufruf fehlgeschlagen (verdict=%s)", int(verdict_id))
        return False
    finally:
        if owns_session:
            await session.close()


# --------------------------------------------------------------------------- #
# Persistente Discord-View
# --------------------------------------------------------------------------- #


def build_scam_revoke_custom_id(verdict_id: int) -> str:
    """Stabile custom_id pro Verdict (Voraussetzung für persistente Views)."""
    return f"scam-revoke:{int(verdict_id)}"


class _ScamRevokeButton(discord.ui.Button):
    def __init__(self, parent: ScamRevokeView, *, custom_id: str) -> None:
        super().__init__(
            label=_BUTTON_LABEL_ACTIVE,
            style=discord.ButtonStyle.danger,
            custom_id=custom_id,
        )
        self._parent = parent

    async def callback(self, interaction: discord.Interaction) -> None:
        await self._parent.handle_click(interaction)


class ScamRevokeView(discord.ui.View):
    """Persistenter „Rückgängig"-Button an einer Scam-Meldung."""

    def __init__(
        self,
        *,
        verdict_id: int,
        channel_login: str,
        chatter_login: str,
        action_taken: str,
    ) -> None:
        super().__init__(timeout=None)
        self.verdict_id = int(verdict_id)
        self.channel_login = str(channel_login)
        self.chatter_login = str(chatter_login)
        self.action_taken = str(action_taken)
        self.channel_id: int | None = None
        self.message_id: int | None = None
        self.add_item(
            _ScamRevokeButton(self, custom_id=build_scam_revoke_custom_id(self.verdict_id))
        )

    def bind_to_message(self, *, channel_id: int | None, message_id: int | None) -> None:
        self.channel_id = channel_id if channel_id and channel_id > 0 else None
        self.message_id = message_id if message_id and message_id > 0 else None

    def _mark_revoked(self) -> None:
        for child in self.children:
            child.disabled = True
            if isinstance(child, discord.ui.Button):
                child.label = _BUTTON_LABEL_REVOKED
                child.style = discord.ButtonStyle.secondary

    async def handle_click(self, interaction: discord.Interaction) -> None:
        try:
            if not interaction.response.is_done():
                await interaction.response.defer()
        except Exception:
            logger.debug("scam-revoke: defer fehlgeschlagen", exc_info=True)

        ok = await post_revoke(self.verdict_id)
        if not ok:
            await self._safe_followup(interaction, _FOLLOWUP_FAILURE)
            return

        self._mark_revoked()
        try:
            await interaction.edit_original_response(view=self)
        except Exception:
            logger.exception(
                "scam-revoke: Nachricht konnte nach Rücknahme nicht editiert werden (verdict=%s)",
                self.verdict_id,
            )
        await self._safe_followup(interaction, _FOLLOWUP_SUCCESS)

    @staticmethod
    async def _safe_followup(interaction: discord.Interaction, content: str) -> None:
        try:
            await interaction.followup.send(content, ephemeral=True)
        except Exception:
            logger.debug("scam-revoke: Followup-Nachricht fehlgeschlagen", exc_info=True)


def build_scam_revoke_view(view_spec: dict[str, Any]) -> ScamRevokeView:
    """Baut die View aus einem bereits validierten `scam_revoke`-view_spec."""
    return ScamRevokeView(
        verdict_id=int(view_spec["verdict_id"]),
        channel_login=str(view_spec.get("channel_login") or ""),
        chatter_login=str(view_spec.get("chatter_login") or ""),
        action_taken=str(view_spec.get("action_taken") or ""),
    )
