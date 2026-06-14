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
import json
import logging
import os
from typing import Any

import aiohttp
import discord
from discord import app_commands
from discord.ext import commands

from service import db

log = logging.getLogger(__name__)

# kv_store-Referenz der geposteten Panel-Message — gleicher Namespace wie die
# alte Python-Implementierung (account_link_panel.py), damit ein bereits
# gespeichertes Panel nahtlos übernommen wird.
_PANEL_KV_NS = "steam_link_panel"
_PANEL_KV_KEY = "panel_ref"


async def _get_stored_panel_ref() -> tuple[int, int] | None:
    row = await db.query_one_async(
        "SELECT v FROM kv_store WHERE ns = ? AND k = ?",
        (_PANEL_KV_NS, _PANEL_KV_KEY),
    )
    if not row:
        return None
    try:
        raw = row[0] if not isinstance(row, dict) else row.get("v")
        payload = json.loads(raw)
        return int(payload["channel_id"]), int(payload["message_id"])
    except Exception:
        return None


async def _store_panel_ref(channel_id: int, message_id: int) -> None:
    await db.execute_async(
        "INSERT OR REPLACE INTO kv_store (ns, k, v) VALUES (?, ?, ?)",
        (
            _PANEL_KV_NS,
            _PANEL_KV_KEY,
            json.dumps({"channel_id": int(channel_id), "message_id": int(message_id)}),
        ),
    )


async def _clear_panel_ref() -> None:
    await db.execute_async(
        "DELETE FROM kv_store WHERE ns = ? AND k = ?",
        (_PANEL_KV_NS, _PANEL_KV_KEY),
    )

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

# Der Rang-Abruf beschäftigt den steam-core (Profilkarte vom Game-Coordinator)
# mehrere Sekunden. Diese Buttons brauchen darum ein größeres Antwort-Budget als
# den 10s-Default — sonst läuft der Forward ins Timeout, bevor der Rang da ist.
_RANKCHECK_IDS: frozenset[str] = frozenset(
    [
        "steam_link_panel:rankcheck",
        "linkpanel_rank_check",
    ]
)
_RANKCHECK_FORWARD_TIMEOUT = 120.0

# Wartezeit in Sekunden, nach der wir defer + followup statt direkter Response nutzen
_DEFER_THRESHOLD_SECONDS = 2.0

# Admin-Text-Commands (Prefix !)
_ADMIN_COMMANDS: frozenset[str] = frozenset(
    [
        "!steam_status",
        "!steam_friend_request",
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

    def __init__(
        self,
        custom_id: str,
        label: str = "​",  # Zero-Width-Space als Default für bereits gepostete Panels
        style: discord.ButtonStyle = discord.ButtonStyle.secondary,
    ) -> None:
        super().__init__(
            label=label,
            custom_id=custom_id,
            style=style,
        )

    async def callback(self, interaction: discord.Interaction) -> None:
        await _forward_interaction(interaction, self.custom_id)


_PANEL_BUTTON_MAP: dict[str, tuple[str, discord.ButtonStyle]] = {
    "steam_link_panel:open": ("🔗 Steam verknüpfen", discord.ButtonStyle.primary),
    "steam_link_panel:friend_code": ("🔢 Freundescode eingeben", discord.ButtonStyle.secondary),
    "steam_link_panel:rankcheck": ("📊 Rang prüfen", discord.ButtonStyle.secondary),
}


class SteamBridgePanelView(discord.ui.View):
    """Persistente View, die alle Link-Panel-Buttons abfängt (timeout=None)."""

    def __init__(self) -> None:
        super().__init__(timeout=None)
        for cid in _PANEL_CUSTOM_IDS:
            label_style = _PANEL_BUTTON_MAP.get(cid)
            if label_style:
                label, style = label_style
                self.add_item(_PanelButton(custom_id=cid, label=label, style=style))
            else:
                self.add_item(_PanelButton(custom_id=cid))


# ---------------------------------------------------------------------------
# Playtest-Funnel: persistente Views für alle betainvite:* custom_ids
# ---------------------------------------------------------------------------


class _BetaInviteButton(discord.ui.Button):
    """Proxy-Button für einen betainvite:*-custom_id.

    Leitet jeden Klick als kind="interaction" an den Rust-steam-bot weiter.
    Label und Stil können optional übergeben werden; Default bleibt Zero-Width-Space/
    secondary damit alt gepostete persistente Views weiterhin funktionieren.
    """

    def __init__(
        self,
        custom_id: str,
        label: str = "​",  # Zero-Width-Space als Default für persistente Views
        style: discord.ButtonStyle = discord.ButtonStyle.secondary,
    ) -> None:
        super().__init__(
            label=label,
            custom_id=custom_id,
            style=style,
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
        self.add_item(_BetaInviteButton(
            custom_id=_BETAINVITE_PANEL_CUSTOM_ID,
            label="🎟️ Einladung starten",
            style=discord.ButtonStyle.primary,
        ))


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


async def _post_event(
    kind: str, data: dict[str, Any], *, timeout: float = 10.0
) -> dict[str, Any] | None:
    """Sendet ein Event an POST /events/discord des steam-bot.

    Gibt das geparste JSON-Dict zurück, oder None bei Fehler. `timeout` ist
    grosszügiger zu wählen für lang laufende Commands (z.B. ein voller
    Rank-/Friend-Sync über alle Freunde).
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
                timeout=aiohttp.ClientTimeout(total=timeout),
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


async def fetch_steam_link_url(user_id: int) -> str | None:
    """Holt eine frische Einmal-Login-URL (15 min gültig) vom Rust-steam-bot.

    Öffentlicher Helfer für andere Cogs (z. B. steam_link_voice_nudge), damit
    der Wire-Vertrag zum steam-bot an einer Stelle bleibt. Nutzt denselben
    Pfad wie der Panel-Button `steam_link_panel:open`.
    """
    result = await _post_event(
        "interaction",
        {
            "interaction": {
                "custom_id": "steam_link_panel:open",
                "user_id": int(user_id),
                "guild_id": 0,
                "channel_id": 0,
            }
        },
    )
    if not result:
        return None
    link_button = result.get("link_button")
    if isinstance(link_button, dict):
        url = str(link_button.get("url") or "")
        return url or None
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
            view = discord.ui.View(timeout=None)
            view.add_item(
                discord.ui.Button(
                    style=discord.ButtonStyle.link,
                    label=label,
                    url=url,
                )
            )

    # Interaktive Buttons aus der Rust-Antwort (custom_id + label + style)
    _style_map: dict[str, discord.ButtonStyle] = {
        "primary": discord.ButtonStyle.primary,
        "secondary": discord.ButtonStyle.secondary,
        "success": discord.ButtonStyle.success,
        "danger": discord.ButtonStyle.danger,
    }
    buttons_data = result.get("buttons")
    if buttons_data and isinstance(buttons_data, list):
        if view is None:
            view = discord.ui.View(timeout=None)
        for btn_spec in buttons_data:
            if not isinstance(btn_spec, dict):
                continue
            btn_custom_id = str(btn_spec.get("custom_id") or "")
            btn_label = str(btn_spec.get("label") or "​")
            btn_style_name = str(btn_spec.get("style") or "secondary").lower()
            btn_style = _style_map.get(btn_style_name, discord.ButtonStyle.secondary)
            if btn_custom_id:
                view.add_item(_BetaInviteButton(
                    custom_id=btn_custom_id,
                    label=btn_label,
                    style=btn_style,
                ))

    # discord.py erwartet MISSING (nicht None) als "kein View". view=None löst
    # sonst einen TypeError aus und die Antwort geht nie raus — betraf alle
    # Button-losen Antworten wie /checkrank, /steam_rank, whoami, Admin-Syncs.
    view_arg = view if view is not None else discord.utils.MISSING

    if already_deferred:
        await interaction.followup.send(
            content=reply_text,
            embed=embed,
            view=view_arg,
            ephemeral=ephemeral,
        )
    else:
        await interaction.response.send_message(
            content=reply_text,
            embed=embed,
            view=view_arg,
            ephemeral=ephemeral,
        )


def _slash_interaction(
    interaction: discord.Interaction,
    name: str,
    options: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Baut das `interaction`-Wire-Objekt für einen Slash-Command.

    Format-Vertrag mit der Rust-Seite (events.rs `dispatch_slash_command`):
    `data.name` = Command-Name, `data.options.<key>` = Argumente.
    """
    data: dict[str, Any] = {"name": name}
    if options:
        data["options"] = options
    return {
        "custom_id": "",
        "user_id": interaction.user.id,
        "guild_id": getattr(interaction.guild, "id", None) or 0,
        "data": data,
    }


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
    # Select-Menü-Werte, falls vorhanden. interaction.data ist ein dict —
    # getattr(...) liefert sonst die eingebaute dict.values-Methode.
    data = getattr(interaction, "data", None)
    values = data.get("values") if isinstance(data, dict) else None
    if isinstance(values, (list, tuple)) and values:
        inner["values"] = list(values)
    # Anzeigename mitsenden, damit der Rust-Funnel ihn z.B. für den Supporter-
    # Namen nutzen kann statt der numerischen ID (liest payload.data.discord_name).
    inner["data"] = {"discord_name": interaction.user.display_name}
    event_data: dict[str, Any] = {"interaction": inner}

    # Wir starten den API-Call und warten maximal _DEFER_THRESHOLD_SECONDS.
    # Dauert er länger, defer wir die Interaction und senden dann ein Followup.
    # Rang-Abrufe bekommen ein größeres Budget (Profilkarten-Lookup über steam-core).
    if custom_id in _RANKCHECK_IDS:
        task = asyncio.create_task(
            _post_event("interaction", event_data, timeout=_RANKCHECK_FORWARD_TIMEOUT)
        )
    else:
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
        # Panel-Restore läuft nach Ready: gespeicherte Panel-Message mit dem
        # aktuellen Embed aus dem Rust-Bot auffrischen (wie account_link_panel).
        asyncio.create_task(self._restore_panel())

    async def _fetch_steam_panel_embed(self, user_id: int) -> discord.Embed | None:
        """Holt das aktuelle Steam-Link-Panel-Embed vom Rust-steam-bot."""
        result = await _post_event(
            "slash_command",
            {
                "interaction": {
                    "custom_id": "",
                    "user_id": user_id,
                    "guild_id": 0,
                    "data": {"name": "publish_steam_panel"},
                }
            },
        )
        if result is None:
            return None
        embed_dict = result.get("reply_embed")
        if embed_dict and isinstance(embed_dict, dict):
            try:
                return discord.Embed.from_dict(embed_dict)
            except Exception as exc:
                log.warning("steam-bridge: Konnte Steam-Panel-Embed nicht parsen: %s", exc)
        return None

    async def _restore_panel(self) -> None:
        """Frischt die gespeicherte Panel-Message nach einem Restart auf."""
        await self.bot.wait_until_ready()
        ref = await _get_stored_panel_ref()
        if not ref:
            return
        channel_id, message_id = ref

        channel = self.bot.get_channel(channel_id)
        if channel is None:
            try:
                channel = await self.bot.fetch_channel(channel_id)
            except Exception:
                log.warning("steam-bridge: Panel-Channel %s nicht gefunden.", channel_id)
                return
        if not isinstance(channel, (discord.TextChannel, discord.Thread)):
            log.warning("steam-bridge: Panel-Channel %s ist kein Textkanal.", channel_id)
            return

        try:
            message = await channel.fetch_message(message_id)
        except discord.NotFound:
            await _clear_panel_ref()
            return
        except Exception:
            log.warning("steam-bridge: Panel-Message %s nicht ladbar.", message_id, exc_info=True)
            return

        embed = await self._fetch_steam_panel_embed(self.bot.user.id)
        if embed is None:
            # Rust-Bot (noch) nicht erreichbar — Panel unangetastet lassen,
            # die persistente View funktioniert auch ohne Embed-Refresh.
            return
        try:
            await message.edit(embed=embed, view=SteamBridgePanelView())
            log.info("steam-bridge: Panel-Message %s aufgefrischt.", message_id)
        except Exception:
            log.warning("steam-bridge: Panel-Message %s nicht aktualisierbar.", message_id, exc_info=True)

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
    @app_commands.describe(channel="Zielkanal fürs Panel (Standard: aktueller Kanal)")
    async def publish_betainvite_panel(
        self,
        interaction: discord.Interaction,
        channel: discord.TextChannel | discord.Thread | None = None,
    ) -> None:
        """Postet das persistente BetaInvite-Panel (Zielkanal wählbar).

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

        if channel is not None:
            # Panel in den gewählten Kanal posten, Bestätigung ephemer.
            await channel.send(content=reply_text, embed=embed, view=view)
            await interaction.response.send_message(
                f"✅ Invite-Panel in {channel.mention} gepostet.", ephemeral=True
            )
            return

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
    # Slash-Commands: Account-Verknüpfung
    # ------------------------------------------------------------------

    @app_commands.command(
        name="account_verknüpfen",
        description="Zeigt den Steam-OpenID-Link zur Account-Verknüpfung.",
    )
    async def account_verknuepfen(self, interaction: discord.Interaction) -> None:
        """Einstieg in den Link-Flow (= Panel-Button steam_link_panel:open)."""
        result = await _post_event(
            "slash_command",
            {"interaction": _slash_interaction(interaction, "account_verknüpfen")},
        )
        await _render_response(interaction, result)

    steam = app_commands.Group(name="steam", description="Steam-Links verwalten")

    @steam.command(name="links", description="Zeigt deine gespeicherten Steam-Links.")
    async def steam_links(self, interaction: discord.Interaction) -> None:
        result = await _post_event(
            "slash_command",
            {"interaction": _slash_interaction(interaction, "steam_links")},
        )
        await _render_response(interaction, result)

    @steam.command(
        name="whoami",
        description="Prüft ID/Vanity/Profil-Link und zeigt Persona + SteamID.",
    )
    @app_commands.describe(steam="SteamID64, Vanity oder steamcommunity-Link")
    async def steam_whoami(self, interaction: discord.Interaction, steam: str) -> None:
        result = await _post_event(
            "slash_command",
            {"interaction": _slash_interaction(interaction, "steam_whoami", {"steam": steam})},
        )
        await _render_response(interaction, result)

    @steam.command(
        name="setprimary",
        description="Markiert einen verknüpften Steam-Account als Primär.",
    )
    @app_commands.describe(
        steam="SteamID64, Vanity oder steamcommunity-Link",
        name="Optionaler Anzeigename",
    )
    async def steam_setprimary(
        self, interaction: discord.Interaction, steam: str, name: str | None = None
    ) -> None:
        options: dict[str, Any] = {"steam": steam}
        if name:
            options["name"] = name
        result = await _post_event(
            "slash_command",
            {"interaction": _slash_interaction(interaction, "steam_setprimary", options)},
        )
        await _render_response(interaction, result)

    @steam.command(name="unlink", description="Entfernt einen Steam-Link.")
    @app_commands.describe(steam="SteamID64, Vanity oder steamcommunity-Link")
    async def steam_unlink(self, interaction: discord.Interaction, steam: str) -> None:
        result = await _post_event(
            "slash_command",
            {"interaction": _slash_interaction(interaction, "steam_unlink", {"steam": steam})},
        )
        await _render_response(interaction, result)

    # ------------------------------------------------------------------
    # Slash-Commands: Rang-Abfrage
    # ------------------------------------------------------------------

    @app_commands.command(
        name="steam_rank",
        description="Fragt den Deadlock-Rang über die Steam PlayerCard ab.",
    )
    @app_commands.describe(target="SteamID64/Vanity/Link (leer = dein eigener Account)")
    async def steam_rank(
        self, interaction: discord.Interaction, target: str | None = None
    ) -> None:
        await interaction.response.defer(ephemeral=True, thinking=True)
        options = {"target": target} if target else {}
        result = await _post_event(
            "slash_command",
            {"interaction": _slash_interaction(interaction, "steam_rank", options)},
            timeout=60.0,
        )
        await _render_response(interaction, result, already_deferred=True)

    @app_commands.command(
        name="checkrank",
        description="Prüft den Deadlock-Rang eines Discord-Users per @Mention.",
    )
    @app_commands.describe(user="Discord-User (leer = du selbst)")
    async def checkrank(
        self, interaction: discord.Interaction, user: discord.Member | None = None
    ) -> None:
        await interaction.response.defer(ephemeral=True, thinking=True)
        target = user or interaction.user
        options = {
            "target_user_id": target.id,
            "target_mention": getattr(target, "mention", f"`{target}`"),
        }
        result = await _post_event(
            "slash_command",
            {"interaction": _slash_interaction(interaction, "checkrank", options)},
            timeout=120.0,
        )
        await _render_response(interaction, result, already_deferred=True)

    # ------------------------------------------------------------------
    # Slash-Commands: Admin-Sync
    # ------------------------------------------------------------------

    @app_commands.command(
        name="steam_rank_sync",
        description="(Admin) Synchronisiert Friend-Ranks und Rang-Rollen.",
    )
    @app_commands.checks.has_permissions(administrator=True)
    async def steam_rank_sync(self, interaction: discord.Interaction) -> None:
        await self._run_admin_sync(interaction, "steam_rank_sync")

    @app_commands.command(
        name="subrank_sync",
        description="(Admin) Startet sofort den Deadlock Subrank-Auto-Sync.",
    )
    @app_commands.checks.has_permissions(administrator=True)
    async def subrank_sync(self, interaction: discord.Interaction) -> None:
        await self._run_admin_sync(interaction, "subrank_sync")

    @app_commands.command(
        name="sync_steam_friends",
        description="(Admin) Synchronisiert die Steam-Freundesliste + Verified-Rollen.",
    )
    @app_commands.checks.has_permissions(administrator=True)
    async def sync_steam_friends(self, interaction: discord.Interaction) -> None:
        await self._run_admin_sync(interaction, "sync_steam_friends")

    async def _run_admin_sync(
        self, interaction: discord.Interaction, command_name: str
    ) -> None:
        """Gemeinsamer Pfad für die lang laufenden Admin-Sync-Commands."""
        await interaction.response.defer(ephemeral=True, thinking=True)
        result = await _post_event(
            "slash_command",
            {"interaction": _slash_interaction(interaction, command_name)},
            timeout=180.0,
        )
        await _render_response(interaction, result, already_deferred=True)

    @app_commands.command(
        name="publish_steam_panel",
        description="(Admin) Steam-Verknüpfen-Panel in diesem Channel posten / aktualisieren.",
    )
    @app_commands.checks.has_permissions(administrator=True)
    @app_commands.describe(
        message_id="ID einer bestehenden Message, die editiert werden soll (optional)"
    )
    async def publish_steam_panel(
        self, interaction: discord.Interaction, message_id: str | None = None
    ) -> None:
        """Postet/aktualisiert das persistente Steam-Link-Panel (Rust liefert das Embed)."""
        embed = await self._fetch_steam_panel_embed(interaction.user.id)
        if embed is None:
            await interaction.response.send_message(
                "⚠️ Steam-Bot ist gerade nicht erreichbar. Bitte erneut versuchen.",
                ephemeral=True,
            )
            return

        view = SteamBridgePanelView()

        # Explizit angegebene Message editieren?
        if message_id:
            try:
                mid = int(message_id)
                msg = await interaction.channel.fetch_message(mid)
                await msg.edit(embed=embed, view=view)
                await _store_panel_ref(interaction.channel.id, msg.id)
                await interaction.response.send_message(
                    f"✅ Panel aktualisiert: `{msg.id}`", ephemeral=True
                )
                return
            except (ValueError, discord.NotFound):
                await interaction.response.send_message(
                    "❌ Message nicht gefunden. Neues Panel wird gepostet.", ephemeral=True
                )
            except discord.Forbidden:
                await interaction.response.send_message(
                    "❌ Keine Berechtigung, diese Message zu editieren.", ephemeral=True
                )
                return

        # Gespeicherte Panel-Message im selben Kanal? Dann editieren statt doppelt posten.
        stored_ref = await _get_stored_panel_ref()
        if stored_ref and stored_ref[0] == interaction.channel.id:
            try:
                msg = await interaction.channel.fetch_message(stored_ref[1])
                await msg.edit(embed=embed, view=view)
                if not interaction.response.is_done():
                    await interaction.response.send_message(
                        f"✅ Panel aktualisiert: `{msg.id}`", ephemeral=True
                    )
                return
            except discord.NotFound:
                await _clear_panel_ref()

        # Neues Panel posten + Referenz für den Restart-Restore speichern.
        msg = await interaction.channel.send(embed=embed, view=view)
        await _store_panel_ref(interaction.channel.id, msg.id)
        if not interaction.response.is_done():
            await interaction.response.send_message("✅ Steam-Panel gepostet.", ephemeral=True)

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
        # timeout > 15s: steam_status antwortet synchron und wartet Rust-seitig
        # bis zu 15s auf den AUTH_STATUS-Task.
        result = await _post_event(
            "admin_command",
            {
                "admin_command": {
                    "name": command.lstrip("!"),
                    "args": " ".join(parts[1:]),
                    "invoker_id": message.author.id,
                }
            },
            timeout=30.0,
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
