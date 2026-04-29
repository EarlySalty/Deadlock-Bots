from __future__ import annotations

import asyncio
import logging
import os
import sqlite3
from collections.abc import Iterable
from datetime import datetime

import discord
from discord.ext import commands

from cogs import privacy_core as privacy
from cogs.steam.friend_requests import queue_friend_request
from cogs.steam.logging_utils import safe_log_extra
from cogs.welcome_dm.step_steam_link import steam_link_detailed_description
from service import db

log = logging.getLogger("SteamVoiceNudge")

# ---------- Einstellungen ----------
MIN_VOICE_MINUTES = 30  # Mindest-Verweildauer im Voice (einmalig)
POLL_INTERVAL = 15  # Sekunden – Voice-Alive-Check
DEFAULT_TEST_TARGET_ID = int(os.getenv("NUDGE_TEST_DEFAULT_ID", "0"))
LOG_CHANNEL_ID = 1374364800817303632  # Meldungen in diesen Kanal posten
NUDGE_VIEW_VERSION = 2  # Version der persistierten DM-View
VOICE_NUDGE_FIRST_SEEN_NS = "voice_nudge_first_seen"
VOICE_NUDGE_DONE_NS = "voice_nudge_done"

# Rollen mit Opt-Out (werden NICHT kontaktiert)
# Standard enthält die gewünschte English-only Rolle: 1309741866098491479
_EXEMPT_DEFAULT = "1309741866098491479"
EXEMPT_ROLE_IDS = {
    int(x)
    for x in os.getenv("NUDGE_EXEMPT_ROLE_IDS", _EXEMPT_DEFAULT).split(",")
    if x.strip().isdigit()
}


def _today_str() -> str:
    return datetime.utcnow().date().isoformat()


def _get_first_voice_seen(user_id: int) -> str | None:
    try:
        return db.get_kv(VOICE_NUDGE_FIRST_SEEN_NS, str(int(user_id)))
    except Exception:
        return None


def _remember_first_voice_seen(user_id: int) -> None:
    try:
        db.set_kv(VOICE_NUDGE_FIRST_SEEN_NS, str(int(user_id)), _today_str())
    except Exception:
        log.debug("[nudge] Konnte first-seen nicht speichern", exc_info=True)


def _mark_nudge_done(user_id: int, status: str = "sent") -> None:
    try:
        db.set_kv(VOICE_NUDGE_DONE_NS, str(int(user_id)), status)
    except Exception:
        log.warning("[nudge] Konnte Nudge-Done-Flag nicht setzen (user_id=%s)", user_id, exc_info=True)


def _is_nudge_done(user_id: int) -> bool:
    try:
        return bool(db.get_kv(VOICE_NUDGE_DONE_NS, str(int(user_id))))
    except Exception:
        return False


# ---------- DB ----------
def _save_steam_link_row(user_id: int, steam_id: str, name: str = "", verified: int = 0) -> None:
    _ensure_schema()
    db.execute(
        """
        INSERT INTO steam_links(user_id, steam_id, name, verified)
        VALUES(?,?,?,?)
        ON CONFLICT(user_id, steam_id) DO UPDATE SET
          name=excluded.name,
          verified=excluded.verified,
          updated_at=CURRENT_TIMESTAMP
        """,
        (int(user_id), str(steam_id), name or "", int(verified)),
    )
    try:
        queue_friend_request(steam_id)
    except Exception:
        log.exception(
            "[nudge] Konnte Steam-Freundschaftsanfrage nicht einreihen",
            extra=safe_log_extra({"steam_id": steam_id}),
        )


def _ensure_schema():
    db.execute("""
        CREATE TABLE IF NOT EXISTS steam_nudge_state(
          user_id     INTEGER PRIMARY KEY,
          notified_at DATETIME,
          first_seen  DATETIME DEFAULT CURRENT_TIMESTAMP,
          message_id  INTEGER,
          channel_id  INTEGER,
          view_version INTEGER DEFAULT 0
        )
    """)
    for sql in (
        "ALTER TABLE steam_nudge_state ADD COLUMN message_id INTEGER",
        "ALTER TABLE steam_nudge_state ADD COLUMN channel_id INTEGER",
        "ALTER TABLE steam_nudge_state ADD COLUMN view_version INTEGER DEFAULT 0",
    ):
        try:
            db.execute(sql)
        except sqlite3.OperationalError as exc:
            # Nur loggen, wenn es NICHT "duplicate column" ist (das ist erwartet)
            if "duplicate column" not in str(exc).lower():
                log.debug("[nudge] Konnte Schema-Änderung nicht anwenden (%s): %s", sql, exc)
    db.execute("""
        CREATE TABLE IF NOT EXISTS steam_links(
          user_id         INTEGER NOT NULL,
          steam_id        TEXT    NOT NULL,
          name            TEXT,
          verified        INTEGER DEFAULT 0,
          primary_account INTEGER DEFAULT 0,
          created_at      DATETIME DEFAULT CURRENT_TIMESTAMP,
          updated_at      DATETIME DEFAULT CURRENT_TIMESTAMP,
          PRIMARY KEY (user_id, steam_id)
        )
    """)


def _has_any_steam_link(user_id: int) -> bool:
    # _ensure_schema() -> moved to cog_load
    row = db.query_one("SELECT 1 FROM steam_links WHERE user_id=? LIMIT 1", (int(user_id),))
    return bool(row)


def _load_nudge_state(user_id: int) -> sqlite3.Row | None:
    # _ensure_schema() -> moved to cog_load
    return db.query_one(
        "SELECT user_id, notified_at, first_seen, message_id, channel_id, view_version FROM steam_nudge_state WHERE user_id=?",
        (int(user_id),),
    )


def _iter_nudge_states() -> Iterable[sqlite3.Row]:
    # _ensure_schema() -> moved to cog_load
    return db.query_all(
        "SELECT user_id, notified_at, first_seen, message_id, channel_id, view_version FROM steam_nudge_state",
    )


def _mark_notified(
    user_id: int,
    *,
    message_id: int | None,
    channel_id: int | None,
    view_version: int = NUDGE_VIEW_VERSION,
) -> None:
    # _ensure_schema() -> moved to cog_load
    db.execute(
        """
        INSERT INTO steam_nudge_state(user_id, notified_at, message_id, channel_id, view_version)
        VALUES(?, CURRENT_TIMESTAMP, ?, ?, ?)
        ON CONFLICT(user_id) DO UPDATE SET
          notified_at=excluded.notified_at,
          message_id=excluded.message_id,
          channel_id=excluded.channel_id,
          view_version=excluded.view_version
        """,
        (int(user_id), message_id, channel_id, view_version),
    )


def _clear_nudge_state(user_id: int) -> None:
    # _ensure_schema() -> moved to cog_load
    db.execute(
        "UPDATE steam_nudge_state SET message_id=NULL, channel_id=NULL, view_version=0 WHERE user_id=?",
        (int(user_id),),
    )


def _member_has_exempt_role(member: discord.Member) -> bool:
    return any((r.id in EXEMPT_ROLE_IDS) for r in getattr(member, "roles", []) or [])


def _log_chan(bot: commands.Bot) -> discord.TextChannel | None:
    ch = bot.get_channel(LOG_CHANNEL_ID)
    return ch if isinstance(ch, discord.TextChannel) else None


# ---------- Voice-Monitor ----------
async def _count_voice_minutes(member: discord.Member, minutes: int) -> bool:
    """
    Wartet bis zu `minutes` Minuten, während der Member (irgendeinem) Voice-Channel angehört.
    Gibt True zurück, wenn er die gesamte Zeit im Voice war (abzügl. kurzer Poll-Gaps),
    sonst False, wenn er vorher abhaut.
    """
    seen = 0
    try:
        while seen < minutes * 60:
            await asyncio.sleep(POLL_INTERVAL)
            vc = getattr(member, "voice", None)
            if not vc or not vc.channel:
                return False
            seen += POLL_INTERVAL
        return True
    except asyncio.CancelledError:
        raise
    except Exception:
        log.exception("voice wait failed for member=%s", getattr(member, "id", "?"))
        return False


# ---------- OAuth/OpenID Hilfen ----------
def _find_steamlink_cog(bot: commands.Bot):
    # 1) explizit
    for name in ("SteamLink", "SteamLinkOAuth", "SteamLinkOpenID"):
        cog = bot.get_cog(name)
        if cog:
            return cog
    # 2) heuristisch
    for name, cog in bot.cogs.items():
        low = name.lower()
        if "steam" in low and ("oauth" in low or "link" in low or "openid" in low):
            return cog
    return None


async def _fetch_oauth_urls(
    bot: commands.Bot, user: discord.User | discord.Member
) -> tuple[str | None, str | None]:
    """
    Holt die gültige Steam-Start-URL vom SteamLink-OAuth-Cog.
    Gibt (None, steam_start_url) zurück oder (None, None) als Fallback.
    """
    cog = _find_steamlink_cog(bot)
    if cog:
        try:
            if hasattr(cog, "steam_start_url_for"):
                s = cog.steam_start_url_for(int(user.id))
            else:
                state = cog._mk_state(int(user.id))  # type: ignore[attr-defined]
                s = cog._build_steam_login_url(state)  # type: ignore[attr-defined]
        except Exception:
            log.exception("fetch steam openid url failed")
            s = None
        return None, s or None
    return None, None


# ---------- View/Modal ----------
class _CloseButton(discord.ui.Button):
    def __init__(self, row: int = 1):
        super().__init__(
            label="Schließen",
            style=discord.ButtonStyle.secondary,
            emoji="❌",
            custom_id="nudge_close",
            row=row,
        )

    async def callback(self, interaction: discord.Interaction):
        try:
            if not interaction.response.is_done():
                await interaction.response.defer(ephemeral=True)
        except asyncio.CancelledError:
            raise
        except Exception as e:
            log.debug("close-button defer failed: %r", e)
        try:
            await interaction.message.delete()
        except asyncio.CancelledError:
            raise
        except Exception as e:
            log.debug("close-button delete failed: %r", e)


class _OptionsView(discord.ui.View):
    """Nicht-persistente Instanz mit der aktuellen Steam-Verknüpfungsoption."""

    def __init__(self, *, steam_url: str | None):
        super().__init__(timeout=None)

        if steam_url:
            self.add_item(
                discord.ui.Button(
                    label="Mit Steam anmelden",
                    style=discord.ButtonStyle.link,
                    url=steam_url,
                    emoji="🎮",
                    row=0,
                )
            )
        else:
            self.add_item(
                discord.ui.Button(
                    label="Mit Steam anmelden",
                    style=discord.ButtonStyle.secondary,
                    disabled=True,
                    emoji="🎮",
                    row=0,
                )
            )

        self.add_item(_CloseButton(row=1))


# Persistente Registry-View (falls weitere Buttons mit custom_id notwendig)
class _PersistentRegistryView(discord.ui.View):
    def __init__(self):
        super().__init__(timeout=None)

    @discord.ui.button(
        label="Schließen",
        style=discord.ButtonStyle.secondary,
        emoji="❌",
        custom_id="nudge_close",
    )
    async def _close(self, interaction: discord.Interaction, button: discord.ui.Button):
        try:
            if not interaction.response.is_done():
                await interaction.response.defer(ephemeral=True)
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            log.debug("[nudge] Close-Button konnte nicht deferen: %s", exc)
        try:
            await interaction.message.delete()
        except Exception as exc:
            log.debug("[nudge] Konnte Interaktionsnachricht nicht löschen: %s", exc)


# ---------- Cog ----------
class SteamLinkVoiceNudge(commands.Cog):
    """
    Nudge-Nachricht in DMs, wenn Member lange genug im Voice war und noch kein Steam-Link hinterlegt ist.
    Mit Rollen-Opt-Out, Logging und robustem Cleanup.
    """

    def __init__(self, bot: commands.Bot):
        self.bot = bot
        self._tasks: dict[int, asyncio.Task] = {}
        self._restore_task: asyncio.Task | None = None

    async def cog_load(self):
        # Schema einmalig beim Start sicherstellen (Performance & Spam-Vermeidung)
        try:
            _ensure_schema()
        except Exception:
            log.exception("[nudge] Schema-Init fehlgeschlagen")

        self.bot.add_view(_PersistentRegistryView())
        try:
            self._restore_task = asyncio.create_task(self._restore_persistent_messages())
        except Exception:
            log.exception("[nudge] Konnte Persistenz-Wiederherstellung nicht starten")

    async def cog_unload(self):
        for uid, t in list(self._tasks.items()):
            try:
                t.cancel()
            except Exception as exc:
                log.debug("[nudge] Konnte Task %s nicht canceln: %s", uid, exc)
        self._tasks.clear()
        if self._restore_task:
            try:
                self._restore_task.cancel()
            except Exception as exc:
                log.debug("[nudge] Konnte Restore-Task nicht canceln: %s", exc)
            self._restore_task = None

    async def _restore_persistent_messages(self) -> None:
        try:
            rows = list(_iter_nudge_states())
        except asyncio.CancelledError:
            raise
        except Exception:
            log.exception("[nudge] Konnte Nudge-States nicht laden")
            return

        for row in rows:
            try:
                user_id = int(row["user_id"])
            except Exception:
                continue

            message_id = row.get("message_id") if isinstance(row, dict) else row["message_id"]
            if not message_id:
                continue

            user: discord.Member | discord.User | None = self.bot.get_user(user_id)
            if not user:
                try:
                    user = await self.bot.fetch_user(user_id)
                except (discord.HTTPException, discord.Forbidden, discord.NotFound):
                    user = None
            if not user:
                continue

            message = await self._fetch_nudge_message(user, row)
            if not message:
                _clear_nudge_state(user_id)
                continue

            try:
                embed, view = await self._build_dm_payload(user)
                await message.edit(embed=embed, view=view)
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                log.debug(
                    "[nudge] Persistente DM konnte nicht aktualisiert werden: %r",
                    exc,
                    extra=safe_log_extra({"user_id": user_id}),
                )
                continue

            _mark_notified(
                user_id,
                message_id=message.id,
                channel_id=message.channel.id,
                view_version=NUDGE_VIEW_VERSION,
            )

    async def _build_dm_payload(
        self, user: discord.User | discord.Member
    ) -> tuple[discord.Embed, _OptionsView]:
        _, steam_url = await _fetch_oauth_urls(self.bot, user)

        desc = steam_link_detailed_description()
        if not steam_url:
            desc += "\n\n_Heads-up:_ Der Link-Dienst ist gerade nicht verfügbar. Nutze vorerst **/account_verknüpfen**."

        embed = discord.Embed(
            title="Steam-Verknüpfung empfohlen 🔗",
            description=desc,
            color=discord.Color.blurple(),
        )
        embed.set_footer(
            text="Kurzbefehle: /account_verknüpfen · /steam unlink · /steam setprimary"
        )

        view = _OptionsView(steam_url=steam_url)
        return embed, view

    async def _fetch_nudge_message(
        self,
        user: discord.Member | discord.User,
        state: sqlite3.Row,
    ) -> discord.Message | None:
        message_id = state["message_id"] if "message_id" in state.keys() else None  # type: ignore[index]
        if not message_id:
            return None

        channel_id = state["channel_id"] if "channel_id" in state.keys() else None  # type: ignore[index]
        dm_channel: discord.DMChannel | None = None

        if channel_id:
            channel = self.bot.get_channel(int(channel_id))
            if channel is None:
                try:
                    channel = await self.bot.fetch_channel(int(channel_id))
                except (discord.HTTPException, discord.Forbidden, discord.NotFound):
                    channel = None
            if isinstance(channel, discord.DMChannel):
                dm_channel = channel

        if dm_channel is None:
            try:
                dm_channel = user.dm_channel or await user.create_dm()
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                log.debug("[nudge] DM-Channel konnte nicht geöffnet werden: %r", exc)
                return None

        try:
            return await dm_channel.fetch_message(int(message_id))
        except (discord.NotFound, discord.Forbidden):
            return None
        except discord.HTTPException as exc:
            log.debug("[nudge] DM-Message fetch fehlgeschlagen: %r", exc)
            return None

    async def _has_active_nudge(self, member: discord.Member) -> bool:
        state = _load_nudge_state(member.id)
        if not state:
            return False

        try:
            message_id = state["message_id"] if "message_id" in state.keys() else None  # type: ignore[index]
            notified_at = state["notified_at"] if "notified_at" in state.keys() else None  # type: ignore[index]
        except Exception:
            message_id = None
            notified_at = None

        if not message_id:
            return bool(notified_at)

        message = await self._fetch_nudge_message(member, state)
        if not message:
            return bool(notified_at)

        try:
            stored_version = (
                int(state.get("view_version", 0))
                if hasattr(state, "get")
                else int(state["view_version"] or 0)
            )
        except Exception:
            stored_version = 0

        if stored_version < NUDGE_VIEW_VERSION:
            try:
                embed, view = await self._build_dm_payload(member)
                await message.edit(embed=embed, view=view)
                _mark_notified(
                    member.id,
                    message_id=message.id,
                    channel_id=message.channel.id,
                    view_version=NUDGE_VIEW_VERSION,
                )
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                log.debug(
                    "[nudge] Aktualisierung der vorhandenen Nudge-DM fehlgeschlagen: %r",
                    exc,
                )

        return True

    @commands.Cog.listener()
    async def on_voice_state_update(
        self,
        member: discord.Member,
        before: discord.VoiceState,
        after: discord.VoiceState,
    ):
        try:
            if (before.channel is None) and (after.channel is not None):
                if privacy.is_opted_out(member.id):
                    return
                if _member_has_exempt_role(member):
                    log.debug("[nudge] skip exempt member id=%s", member.id)
                    return
                if _has_any_steam_link(member.id):
                    log.debug("[nudge] user already linked id=%s", member.id)
                    return
                if _is_nudge_done(member.id):
                    log.debug("[nudge] user already nudged id=%s", member.id)
                    return
                if await self._has_active_nudge(member):
                    log.debug("[nudge] user already notified id=%s", member.id)
                    return

                today = _today_str()
                first_seen = _get_first_voice_seen(member.id)
                if not first_seen:
                    _remember_first_voice_seen(member.id)
                    log.debug("[nudge] first voice join recorded id=%s", member.id)
                    return

                if first_seen == today:
                    log.debug("[nudge] same-day join, nudge postponed id=%s", member.id)
                    return
                if member.id in self._tasks and not self._tasks[member.id].done():
                    try:
                        self._tasks[member.id].cancel()
                    except Exception as exc:
                        log.debug(
                            "[nudge] Bestehenden Task für %s konnte nicht abgebrochen werden: %s",
                            member.id,
                            exc,
                        )
                self._tasks[member.id] = asyncio.create_task(self._wait_and_notify(member))
        except Exception:
            log.exception("on_voice_state_update error")

    async def _send_dm_nudge(
        self, user: discord.User | discord.Member, *, force: bool = False
    ) -> bool:
        uid = int(user.id)
        if privacy.is_opted_out(uid):
            return False
        if not force and _has_any_steam_link(uid):
            return False

        try:
            if not force:
                state = _load_nudge_state(uid)
            else:
                state = None

            if state:
                message = await self._fetch_nudge_message(user, state)
                if message:
                    try:
                        stored_version = (
                            int(state.get("view_version", 0))
                            if hasattr(state, "get")
                            else int(state["view_version"] or 0)
                        )
                    except Exception:
                        stored_version = 0

                    if stored_version < NUDGE_VIEW_VERSION:
                        try:
                            embed, view = await self._build_dm_payload(user)
                            await message.edit(embed=embed, view=view)
                            _mark_notified(
                                uid,
                                message_id=message.id,
                                channel_id=message.channel.id,
                                view_version=NUDGE_VIEW_VERSION,
                            )
                        except asyncio.CancelledError:
                            raise
                        except Exception as exc:
                            log.debug(
                                "[nudge] Aktualisierung bestehender Nudge-DM fehlgeschlagen: %r",
                                exc,
                            )
                    return False
                else:
                    _clear_nudge_state(uid)
                    # Wurde bereits benachrichtigt – nicht erneut senden, nur weil die alte Nachricht weg ist.
                    notified_at = state["notified_at"] if "notified_at" in state.keys() else None  # type: ignore[index]
                    if notified_at:
                        return False

            dm = user.dm_channel or await user.create_dm()
            embed, view = await self._build_dm_payload(user)
            message = await dm.send(embed=embed, view=view)

            _mark_notified(
                uid,
                message_id=message.id,
                channel_id=dm.id,
                view_version=NUDGE_VIEW_VERSION,
            )
            _mark_nudge_done(uid, "sent")

            ch = _log_chan(self.bot)
            if ch:
                await ch.send(f"📨 Nudge-DM an **{user}** ({uid}) gesendet.")
            return True

        except asyncio.CancelledError:
            raise
        except discord.Forbidden:
            ch = _log_chan(self.bot)
            if ch:
                await ch.send(f"⚠️ Nudge-DM an **{user}** ({uid}) fehlgeschlagen: DMs deaktiviert.")
            return False
        except Exception as e:
            log.exception("[nudge] Fehler beim Senden der DM")
            ch = _log_chan(self.bot)
            if ch:
                await ch.send(f"❌ Nudge-DM an **{user}** ({uid}) fehlgeschlagen: `{e}`")
            return False

    async def _wait_and_notify(self, member: discord.Member):
        try:
            if _member_has_exempt_role(member):
                ch = _log_chan(self.bot)
                if ch:
                    await ch.send(f"ℹ️ Übersprungen (Exempt): **{member}** ({member.id})")
                return
            if privacy.is_opted_out(member.id):
                return

            ok = await _count_voice_minutes(member, MIN_VOICE_MINUTES)
            if not ok:
                log.debug("[nudge] %s left voice early – abort", member.id)
                return

            if _has_any_steam_link(member.id):
                log.debug("[nudge] already linked after wait, skip")
                return

            await self._send_dm_nudge(member)

        except asyncio.CancelledError:
            raise
        except Exception:
            log.exception("wait_and_notify failed")

    @commands.hybrid_command(
        name="nudgesend",
        description="(Admin) Schickt die Steam-Nudge-DM an einen Nutzer.",
        aliases=("t30",),
    )
    @commands.has_permissions(administrator=True)
    async def nudgesend(
        self,
        ctx: commands.Context,
        target: discord.Member | discord.User | None = None,
    ):
        target = await self._resolve_test_target(ctx, target)
        if not target:
            await ctx.reply("Bitte Ziel angeben: `!nudgesend @user`", mention_author=False)
            return

        if privacy.is_opted_out(target.id):
            await ctx.reply(
                "⚠️ Nutzer hat ein Opt-out aktiviert; keine Nudge-DM gesendet.",
                mention_author=False,
            )
            return

        if isinstance(target, discord.Member) and _member_has_exempt_role(target):
            await ctx.reply(
                "ℹ️ Test abgebrochen: Ziel hat eine ausgenommene Rolle.",
                mention_author=False,
            )
            return

        ok = await self._send_dm_nudge(target, force=True)
        if ok:
            await ctx.reply(
                f"📨 Test-DM an {getattr(target, 'mention', target.id)} gesendet.",
                mention_author=False,
            )
        else:
            await ctx.reply(
                "⚠️ Test-DM konnte nicht gesendet werden (DMs aus? oder bereits benachrichtigt).",
                mention_author=False,
            )

    async def _resolve_test_target(
        self,
        ctx: commands.Context,
        target: discord.Member | discord.User | None,
    ) -> discord.Member | discord.User | None:
        if target:
            return target

        if not DEFAULT_TEST_TARGET_ID:
            return None

        # Try resolve as guild member first.
        guild = getattr(ctx, "guild", None)
        if guild:
            member = guild.get_member(DEFAULT_TEST_TARGET_ID)
            if member:
                return member
            try:
                member = await guild.fetch_member(DEFAULT_TEST_TARGET_ID)
            except (discord.NotFound, discord.HTTPException, discord.Forbidden):
                member = None
            if member:
                return member

        # Fall back to any known user object in cache/API.
        user = self.bot.get_user(DEFAULT_TEST_TARGET_ID)
        if user:
            return user

        try:
            return await self.bot.fetch_user(DEFAULT_TEST_TARGET_ID)
        except (discord.NotFound, discord.HTTPException, discord.Forbidden):
            return None


async def setup(bot: commands.Bot):
    await bot.add_cog(SteamLinkVoiceNudge(bot))
