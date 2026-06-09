"""Dünner Discord-Arm des Rust-steam-bot.

Leitet Discord-Ereignisse und Button-Interaktionen an den steam-bot HTTP-API weiter
und rendert die Antwort als Discord-Response. Keine Business-Logik in Python.

Env-Variablen:
  STEAM_BOT_API_URL  — Basis-URL des Rust-steam-bot (Standard: http://127.0.0.1:8783)
  TWITCH_INTERNAL_API_TOKEN / MASTER_BROKER_TOKEN / MAIN_BOT_INTERNAL_TOKEN
                     — Auth-Token (erste nicht-leere Variable gewinnt)
"""

from __future__ import annotations

import asyncio
import logging
import os
from typing import Any

import aiohttp
import discord
from discord import app_commands
from discord.ext import commands

log = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Konfiguration
# ---------------------------------------------------------------------------

_STEAM_BOT_API_URL_DEFAULT = "http://127.0.0.1:8783"
_AUTH_TOKEN_ENV_NAMES = (
    "TWITCH_INTERNAL_API_TOKEN",
    "MASTER_BROKER_TOKEN",
    "MAIN_BOT_INTERNAL_TOKEN",
)

# Alle custom_ids der persistenten Link-Panel-Buttons (aus link-ui.md).
# Befund 4: Legacy-IDs alter geposteter Panels ebenfalls registrieren.
_PANEL_CUSTOM_IDS: frozenset[str] = frozenset(
    [
        "steam_link_panel:open",
        "steam_link_panel:friend_code",
        "steam_link_panel:rankcheck",
        # Legacy-IDs (alte persistente Panels)
        "linkpanel_friend_code",
        "linkpanel_rank_check",
    ]
)

# ---------------------------------------------------------------------------
# Playtest-Funnel: alle custom_ids aus dem Beta-Invite-Flow.
# Reihenfolge entspricht den Schritten im Funnel-Spec (nachschlag-funnel.md).
# ---------------------------------------------------------------------------

# Panel-Einstieg (Schritt 0 — persistentes Panel in öffentlichem Kanal)
_BETAINVITE_PANEL_CUSTOM_ID = "betainvite:panel:start"

# Alle betainvite:*-custom_ids, die per Button-Klick an Rust weitergeleitet werden.
# Schritt 0: Intent-Wahl (Community vs. Nur-Einladung)
# Schritt 1: Steam-Link-Prüfung (+ disabled-Placeholder)
# Schritt 2: Freundschaft prüfen
# Schritt 3: Zahlung fortführen / überspringen
# Fehler: erneut versuchen
_BETAINVITE_CUSTOM_IDS: frozenset[str] = frozenset(
    [
        _BETAINVITE_PANEL_CUSTOM_ID,
        "betainvite:intent:community",
        "betainvite:intent:invite_only",
        "betainvite:link:continue",
        "betainvite:link:disabled",
        "betainvite:friendhint:continue",
        "betainvite:payment:continue",
        "betainvite:support:skip",
        "betainvite:error:retry",
    ]
)

# Befund 3+4: custom_ids, die lokal ein Modal öffnen statt an Rust forwardiert zu werden.
_FRIEND_CODE_MODAL_IDS: frozenset[str] = frozenset(
    [
        "steam_link_panel:friend_code",
        "linkpanel_friend_code",
    ]
)

# Wartezeit in Sekunden, nach der wir defer + followup statt direkter Response nutzen
_DEFER_THRESHOLD_SECONDS = 2.0

# Admin-Text-Commands (Prefix !)
_ADMIN_COMMANDS: frozenset[str] = frozenset(
    [
        "!steam_status",
        "!steam_lobby_convar",
        "!steam_lobby_apply",
        "!steam_lobby_event",
        "!steam_lobby_events",
    ]
)

# ---------------------------------------------------------------------------
# Befund 3+4: Freundescode-Modal (lokal, kein Forward an Rust)
# ---------------------------------------------------------------------------

# Texte identisch mit SteamFriendCodeModal aus steam_link_oauth.py
class _FriendCodeModal(discord.ui.Modal, title="Steam-Freundescode"):
    """Öffnet ein Modal zur Eingabe des Steam-Freundescodes.

    Nach Absenden wird die Eingabe per POST /events/discord an den Rust-steam-bot
    weitergeleitet (custom_id steam_link_panel:friend_code:submit).
    """

    friend_code: discord.ui.TextInput = discord.ui.TextInput(
        label="Freundescode",
        placeholder="z. B. 820142646",
        required=True,
        min_length=1,
        max_length=32,
    )

    async def on_submit(self, interaction: discord.Interaction) -> None:
        # Befund 3: interaction-Payload als verschachteltes Objekt senden
        # (kompatibel mit DiscordEventBody.interaction-Feld in Rust)
        result = await _post_event(
            "interaction",
            {
                "interaction": {
                    "custom_id": "steam_link_panel:friend_code:submit",
                    "values": [str(self.friend_code.value or "")],
                    "user_id": interaction.user.id,
                    "guild_id": getattr(interaction.guild, "id", None) or 0,
                    "channel_id": interaction.channel_id or 0,
                }
            },
        )
        await _render_response(interaction, result, already_deferred=False)

    async def on_error(self, interaction: discord.Interaction, error: Exception) -> None:
        log.exception("FriendCode-Modal Fehler (user=%s): %s", interaction.user.id, error)
        msg = "❌ Beim Verarbeiten des Freundescodes ist ein Fehler aufgetreten. Bitte erneut versuchen."
        try:
            if interaction.response.is_done():
                await interaction.followup.send(msg, ephemeral=True)
            else:
                await interaction.response.send_message(msg, ephemeral=True)
        except Exception:
            pass


def _get_token() -> str:
    """Liest den Auth-Token aus der ersten gesetzten Env-Variable."""
    for name in _AUTH_TOKEN_ENV_NAMES:
        value = os.getenv(name, "").strip()
        if value:
            return value
    return ""


def _get_api_url() -> str:
    return os.getenv("STEAM_BOT_API_URL", _STEAM_BOT_API_URL_DEFAULT).rstrip("/")


# ---------------------------------------------------------------------------
# Persistente View: registriert alle Panel-Buttons beim Bot-Start
# ---------------------------------------------------------------------------


class _PanelButton(discord.ui.Button):
    """Einzelner Proxy-Button für den Link-Panel."""

    def __init__(self, custom_id: str) -> None:
        # Label/Stil sind irrelevant — der Rust-bot hat die echte Nachricht gepostet.
        # Hier nur den custom_id-Handler registrieren, damit discord.py
        # Panel-Klicks nach Bot-Restart wieder zuordnen kann.
        super().__init__(
            label="​",  # Zero-Width-Space, unsichtbar
            custom_id=custom_id,
            style=discord.ButtonStyle.secondary,
        )

    async def callback(self, interaction: discord.Interaction) -> None:
        await _forward_interaction(interaction, self.custom_id)


class SteamBridgePanelView(discord.ui.View):
    """Persistente View, die alle Link-Panel-Buttons abfängt (timeout=None)."""

    def __init__(self) -> None:
        super().__init__(timeout=None)
        for cid in _PANEL_CUSTOM_IDS:
            self.add_item(_PanelButton(custom_id=cid))


# ---------------------------------------------------------------------------
# Playtest-Funnel: persistente Views für alle betainvite:* custom_ids
# ---------------------------------------------------------------------------


class _BetaInviteButton(discord.ui.Button):
    """Proxy-Button für einen betainvite:*-custom_id.

    Leitet jeden Klick als kind="interaction" an den Rust-steam-bot weiter.
    Label und Stil werden bei der Registrierung leer/secondary gesetzt —
    die echten Embeds und Beschriftungen kommen vollständig aus Rust.
    """

    def __init__(self, custom_id: str) -> None:
        super().__init__(
            label="​",  # Zero-Width-Space, unsichtbar; echter Label stammt von Rust
            custom_id=custom_id,
            style=discord.ButtonStyle.secondary,
        )

    async def callback(self, interaction: discord.Interaction) -> None:
        await _forward_interaction(interaction, self.custom_id)


class BetaInvitePanelView(discord.ui.View):
    """Persistente View für das Playtest-Invite-Panel (Einstiegs-Button).

    Registriert beim Bot-Start nur den Panel-Einstieg-custom_id
    ("betainvite:panel:start"), damit bereits gepostete Panels nach einem
    Neustart weiterhin auf Klicks reagieren.
    """

    def __init__(self) -> None:
        super().__init__(timeout=None)
        self.add_item(_BetaInviteButton(custom_id=_BETAINVITE_PANEL_CUSTOM_ID))


class BetaInviteFlowView(discord.ui.View):
    """Persistente View, die alle 8 Funnel-Schritt-Buttons abfängt.

    Wird beim Bot-Start registriert, damit laufende Funnel-Sessions nach
    einem Neustart weiterhin funktionieren.  Alle Klicks werden als
    kind="interaction" an den Rust-steam-bot weitergeleitet.
    """

    def __init__(self) -> None:
        super().__init__(timeout=None)
        for cid in _BETAINVITE_CUSTOM_IDS:
            self.add_item(_BetaInviteButton(custom_id=cid))


# ---------------------------------------------------------------------------
# Hilfsfunktionen
# ---------------------------------------------------------------------------


async def _post_event(kind: str, data: dict[str, Any]) -> dict[str, Any] | None:
    """Sendet ein Event an POST /events/discord des steam-bot.

    Gibt das geparste JSON-Dict zurück, oder None bei Fehler.
    """
    url = f"{_get_api_url()}/events/discord"
    token = _get_token()
    headers = {"Content-Type": "application/json"}
    if token:
        headers["X-Internal-Token"] = token

    payload = {"kind": kind, **data}
    try:
        async with aiohttp.ClientSession() as session:
            async with session.post(
                url,
                json=payload,
                headers=headers,
                timeout=aiohttp.ClientTimeout(total=10.0),
            ) as resp:
                if resp.status == 200:
                    return await resp.json()
                body = await resp.text()
                log.warning(
                    "steam-bridge: POST %s → HTTP %s: %s",
                    url,
                    resp.status,
                    body[:200],
                )
                return None
    except asyncio.TimeoutError:
        log.warning("steam-bridge: Timeout beim POST %s", url)
        return None
    except aiohttp.ClientError as exc:
        log.warning("steam-bridge: Verbindungsfehler zu steam-bot: %s", exc)
        return None
    except Exception as exc:
        log.error("steam-bridge: Unerwarteter Fehler beim POST %s: %s", url, exc)
        return None


async def _render_response(
    interaction: discord.Interaction,
    result: dict[str, Any] | None,
    *,
    already_deferred: bool = False,
) -> None:
    """Rendert die Antwort des steam-bot als Discord-Interaction-Response.

    Felder aus result:
      reply_embed   — dict → discord.Embed
      reply_text    — str
      ephemeral     — bool (Standard: True)
      link_button   — dict mit {label, url} → discord.ui.Button(style=link) (Befund 5)
    """
    if result is None:
        msg = "⚠️ Steam-Bot ist gerade nicht erreichbar. Bitte versuche es in wenigen Sekunden erneut."
        if already_deferred:
            await interaction.followup.send(msg, ephemeral=True)
        else:
            await interaction.response.send_message(msg, ephemeral=True)
        return

    ephemeral: bool = bool(result.get("ephemeral", True))
    reply_text: str | None = result.get("reply_text") or None
    embed: discord.Embed | None = None

    embed_dict = result.get("reply_embed")
    if embed_dict and isinstance(embed_dict, dict):
        try:
            embed = discord.Embed.from_dict(embed_dict)
        except Exception as exc:
            log.warning("steam-bridge: Konnte reply_embed nicht parsen: %s", exc)

    # Befund 5: optionaler URL-Button (z.B. für steam_link_panel:open)
    view: discord.ui.View | None = None
    link_button_data = result.get("link_button")
    if link_button_data and isinstance(link_button_data, dict):
        label = str(link_button_data.get("label") or "Öffnen")
        url = str(link_button_data.get("url") or "")
        if url:
            view = discord.ui.View()
            view.add_item(
                discord.ui.Button(
                    style=discord.ButtonStyle.link,
                    label=label,
                    url=url,
                )
            )

    if already_deferred:
        await interaction.followup.send(
            content=reply_text,
            embed=embed,
            view=view,
            ephemeral=ephemeral,
        )
    else:
        await interaction.response.send_message(
            content=reply_text,
            embed=embed,
            view=view,
            ephemeral=ephemeral,
        )


async def _forward_interaction(
    interaction: discord.Interaction, custom_id: str
) -> None:
    """Leitet einen Button-Klick an den steam-bot weiter.

    Befund 3+4: Freundescode-Buttons (steam_link_panel:friend_code, linkpanel_friend_code)
    öffnen lokal ein Modal statt nach Rust forwardiert zu werden.

    Wenn die Antwort länger als _DEFER_THRESHOLD_SECONDS dauert:
    defer() + followup statt direkter Response.
    """
    # Befund 3+4: Freundescode-Buttons öffnen Modal lokal
    if custom_id in _FRIEND_CODE_MODAL_IDS:
        await interaction.response.send_modal(_FriendCodeModal())
        return

    # Kanonisches Wire-Format: Payload verschachtelt im "interaction"-Feld
    # (Rust: DiscordEventBody.interaction → InteractionPayload).
    inner: dict[str, Any] = {
        "custom_id": custom_id,
        "user_id": interaction.user.id,
        "guild_id": getattr(interaction.guild, "id", None) or 0,
        "channel_id": interaction.channel_id or 0,
    }
    # Select-Menü-Werte, falls vorhanden
    values = getattr(getattr(interaction, "data", None), "values", None)
    if values:
        inner["values"] = list(values)
    event_data: dict[str, Any] = {"interaction": inner}

    # Wir starten den API-Call und warten maximal _DEFER_THRESHOLD_SECONDS.
    # Dauert er länger, defer wir die Interaction und senden dann ein Followup.
    task = asyncio.create_task(
        _post_event("interaction", event_data)
    )
    try:
        result = await asyncio.wait_for(asyncio.shield(task), timeout=_DEFER_THRESHOLD_SECONDS)
        await _render_response(interaction, result, already_deferred=False)
        return
    except asyncio.TimeoutError:
        pass

    # Defer — Interaction-Token bleibt 15 Minuten gültig
    try:
        await interaction.response.defer(ephemeral=True)
    except discord.HTTPException:
        pass  # Bereits geantwortet

    result = await task
    await _render_response(interaction, result, already_deferred=True)


# ---------------------------------------------------------------------------
# Cog
# ---------------------------------------------------------------------------


class SteamBridge(commands.Cog, name="SteamBridge"):
    """Dünner Discord-Arm: leitet Events/Interactions an den Rust-steam-bot weiter."""

    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot

    async def cog_load(self) -> None:
        # Persistente Views registrieren — überleben Bot-Restarts
        self.bot.add_view(SteamBridgePanelView())
        self.bot.add_view(BetaInvitePanelView())
        self.bot.add_view(BetaInviteFlowView())
        log.info(
            "steam-bridge: Persistente Views registriert (link=%d, betainvite=%d custom_ids)",
            len(_PANEL_CUSTOM_IDS),
            len(_BETAINVITE_CUSTOM_IDS),
        )

    # ------------------------------------------------------------------
    # Slash-Commands: Playtest-Invite-Funnel
    # ------------------------------------------------------------------

    @app_commands.command(
        name="betainvite",
        description="Starte den Deadlock-Playtest-Invite-Flow.",
    )
    async def betainvite(self, interaction: discord.Interaction) -> None:
        """Einstiegspunkt für alle User — startet den Funnel im Rust-steam-bot."""
        result = await _post_event(
            "slash_command",
            {
                "interaction": {
                    "custom_id": "",
                    "user_id": interaction.user.id,
                    "guild_id": getattr(interaction.guild, "id", None) or 0,
                    "data": {"name": "betainvite"},
                }
            },
        )
        await _render_response(interaction, result)

    @app_commands.command(
        name="publish_betainvite_panel",
        description="Veröffentlicht das Invite-Panel mit dem Einstiegs-Button (nur Admins).",
    )
    @app_commands.checks.has_permissions(manage_guild=True)
    async def publish_betainvite_panel(self, interaction: discord.Interaction) -> None:
        """Postet das persistente BetaInvite-Panel in den aktuellen Kanal.

        Der Rust-steam-bot liefert den vollständigen Embed-Inhalt.
        Python hängt die persistente BetaInvitePanelView an die Nachricht.
        """
        result = await _post_event(
            "slash_command",
            {
                "interaction": {
                    "custom_id": "",
                    "user_id": interaction.user.id,
                    "guild_id": getattr(interaction.guild, "id", None) or 0,
                    "data": {"name": "publish_betainvite_panel"},
                }
            },
        )
        if result is None:
            await interaction.response.send_message(
                "⚠️ Steam-Bot ist gerade nicht erreichbar. Bitte erneut versuchen.",
                ephemeral=True,
            )
            return

        # Rust liefert den Embed; Python fügt die persistente View hinzu
        ephemeral: bool = bool(result.get("ephemeral", False))
        reply_text: str | None = result.get("reply_text") or None
        embed: discord.Embed | None = None
        embed_dict = result.get("reply_embed")
        if embed_dict and isinstance(embed_dict, dict):
            try:
                embed = discord.Embed.from_dict(embed_dict)
            except Exception as exc:
                log.warning("steam-bridge: Konnte reply_embed (publish_panel) nicht parsen: %s", exc)

        # URL-Button-Support wie in _render_response
        view: discord.ui.View = BetaInvitePanelView()
        link_button_data = result.get("link_button")
        if link_button_data and isinstance(link_button_data, dict):
            label = str(link_button_data.get("label") or "Öffnen")
            url = str(link_button_data.get("url") or "")
            if url:
                view.add_item(
                    discord.ui.Button(
                        style=discord.ButtonStyle.link,
                        label=label,
                        url=url,
                    )
                )

        await interaction.response.send_message(
            content=reply_text,
            embed=embed,
            view=view,
            ephemeral=ephemeral,
        )

    @app_commands.command(
        name="betainvite_stats",
        description="Zeigt Funnel-Metriken des Playtest-Invite-Systems (nur Admins).",
    )
    @app_commands.checks.has_permissions(manage_guild=True)
    async def betainvite_stats(self, interaction: discord.Interaction) -> None:
        """Fragt den Rust-steam-bot nach aktuellen Funnel-Statistiken."""
        result = await _post_event(
            "slash_command",
            {
                "interaction": {
                    "custom_id": "",
                    "user_id": interaction.user.id,
                    "guild_id": getattr(interaction.guild, "id", None) or 0,
                    "data": {"name": "betainvite_stats"},
                }
            },
        )
        await _render_response(interaction, result)

    # ------------------------------------------------------------------
    # Member-Ereignisse
    # ------------------------------------------------------------------

    @commands.Cog.listener()
    async def on_member_remove(self, member: discord.Member) -> None:
        """Meldet das Verlassen eines Mitglieds an den steam-bot."""
        await _post_event(
            "member_remove",
            {
                "member_remove": {
                    "guild_id": member.guild.id,
                    "user_id": member.id,
                }
            },
        )

    # ------------------------------------------------------------------
    # Admin-Text-Commands
    # ------------------------------------------------------------------

    @commands.Cog.listener()
    async def on_message(self, message: discord.Message) -> None:
        """Fängt !steam_* Admin-Commands ab und leitet sie weiter."""
        if message.author.bot:
            return
        if not message.guild:
            return

        content = (message.content or "").strip()
        # Schnell-Check: muss mit ! beginnen
        if not content.startswith("!steam_"):
            return

        parts = content.split()
        command = parts[0].lower()
        if command not in _ADMIN_COMMANDS:
            return

        # Admin-Prüfung
        perms = message.author.guild_permissions  # type: ignore[union-attr]
        if not perms.administrator:
            return

        # Rust: AdminCommandPayload {name (ohne '!'), args (Roh-String), invoker_id}
        result = await _post_event(
            "admin_command",
            {
                "admin_command": {
                    "name": command.lstrip("!"),
                    "args": " ".join(parts[1:]),
                    "invoker_id": message.author.id,
                }
            },
        )

        if result is None:
            await message.channel.send(
                "⚠️ Steam-Bot ist gerade nicht erreichbar."
            )
            return

        reply_text: str | None = result.get("reply_text") or None
        embed_dict = result.get("reply_embed")
        embed: discord.Embed | None = None
        if embed_dict and isinstance(embed_dict, dict):
            try:
                embed = discord.Embed.from_dict(embed_dict)
            except Exception as exc:
                log.warning("steam-bridge: Konnte reply_embed (admin_command) nicht parsen: %s", exc)

        await message.channel.send(content=reply_text, embed=embed)


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(SteamBridge(bot))
