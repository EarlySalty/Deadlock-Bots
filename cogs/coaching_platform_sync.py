"""
Coaching Platform Sync Cog

Hält die Website-Coaching-Plattform synchron mit dem Discord-Server:
  1. Rollen-Sync: Mitglieder mit Coach-Rolle → POST coaches/sync (alle 10 Min + on_member_update)
  2. Notification-Poller: fällige Termin-DMs alle 60 Sekunden abholen und zustellen
"""

from __future__ import annotations

import asyncio
import logging
from datetime import datetime, timezone
from zoneinfo import ZoneInfo

import discord
from discord.ext import commands

from service.config import settings
from service import website_client

log = logging.getLogger(__name__)

_BERLIN = ZoneInfo("Europe/Berlin")

_WEEKDAYS_DE = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"]


def _fmt_dt(iso_utc: str) -> str:
    """ISO-UTC-String → 'Mi, 11.06. um 19:00 Uhr' (Europe/Berlin)."""
    try:
        dt_utc = datetime.fromisoformat(iso_utc.replace("Z", "+00:00"))
        dt_berlin = dt_utc.astimezone(_BERLIN)
        wd = _WEEKDAYS_DE[dt_berlin.weekday()]
        return f"{wd}, {dt_berlin.day:02d}.{dt_berlin.month:02d}. um {dt_berlin.hour:02d}:{dt_berlin.minute:02d} Uhr"
    except Exception:
        return iso_utc


def _build_dm_text(item: dict) -> str | None:
    """Baut den DM-Text anhand des notification-Typs. None wenn unbekannter Typ."""
    ntype = item.get("type", "")
    coach = item.get("coach_display", "dein Coach")
    scheduled_at = item.get("scheduled_at", "")
    datum = _fmt_dt(scheduled_at) if scheduled_at else "unbekannter Zeitpunkt"
    duration = item.get("duration_minutes")
    title = item.get("title", "")
    note = item.get("note", "")

    title_line = f"\n{title}" if title else ""
    note_line = f"\n{note}" if note else ""

    if ntype == "created":
        dur_str = f" (ca. {duration} Min.)" if duration else ""
        return (
            f"📅 **Coaching-Termin geplant** — {coach} hat ein Coaching mit dir angesetzt: "
            f"**{datum}**{dur_str}.{title_line}{note_line}\n"
            "Details findest du auf https://deutsche-deadlock-community.de/coaching unter \"Mein Coaching\"."
        )
    if ntype == "reminder":
        return (
            f"⏰ **Erinnerung** — dein Coaching mit {coach} startet **{datum}** (in unter 2 Stunden)."
        )
    if ntype == "cancelled":
        return (
            f"❌ **Termin abgesagt** — dein Coaching mit {coach} am {datum} findet nicht statt. "
            f"Bei Fragen melde dich beim Coach."
        )
    log.warning("Unbekannter Notification-Typ: %s", ntype)
    return None


class CoachingPlatformSyncCog(commands.Cog):
    """Hält Coach-Roster und Termin-DMs zwischen Discord und der Website synchron."""

    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot
        self._role_sync_task: asyncio.Task | None = None
        self._notification_task: asyncio.Task | None = None
        self._debounce_handle: asyncio.TimerHandle | None = None

    # ------------------------------------------------------------------ lifecycle

    async def cog_load(self) -> None:
        if self._role_sync_task is None or self._role_sync_task.done():
            self._role_sync_task = asyncio.create_task(self._role_sync_loop())
        if self._notification_task is None or self._notification_task.done():
            self._notification_task = asyncio.create_task(self._notification_loop())

    async def cog_unload(self) -> None:
        if self._debounce_handle:
            self._debounce_handle.cancel()
            self._debounce_handle = None
        if self._role_sync_task:
            self._role_sync_task.cancel()
            self._role_sync_task = None
        if self._notification_task:
            self._notification_task.cancel()
            self._notification_task = None

    # ------------------------------------------------------------------ role sync

    async def _role_sync_loop(self) -> None:
        await self.bot.wait_until_ready()
        while True:
            try:
                await self._do_role_sync()
            except Exception as exc:
                log.error("Rollen-Sync-Loop Fehler: %s", exc)
            await asyncio.sleep(600)  # alle 10 Minuten

    async def _do_role_sync(self) -> None:
        guild = self.bot.guilds[0] if self.bot.guilds else None
        if not guild:
            log.warning("Keine Guild gefunden – Rollen-Sync übersprungen")
            return

        role = guild.get_role(settings.coach_role_id)
        if not role:
            log.warning("Coach-Rolle (ID %s) nicht gefunden – kein Sync", settings.coach_role_id)
            return

        members = role.members
        if not members:
            # Member-Cache ist direkt nach dem Start oft noch nicht gechunkt → explizit nachladen
            try:
                await guild.chunk()
            except Exception as exc:
                log.warning("Guild-Chunk fehlgeschlagen: %s", exc)
            members = role.members
        if not members:
            log.warning("Coach-Rolle hat keine Mitglieder – Sync wird NICHT ausgeführt (Schutz gegen Roster-Wipe)")
            return

        coaches = [
            {
                "discord_user_id": m.id,
                "discord_username": m.name,
                "display_name": m.display_name,
                "avatar_url": str(m.display_avatar.with_size(256).url),
            }
            for m in members
        ]

        ok = await website_client.sync_coaches(coaches)
        if ok:
            log.info("Coach-Sync erfolgreich: %d Coaches übermittelt", len(coaches))
        else:
            log.warning("Coach-Sync fehlgeschlagen (best-effort, wird beim nächsten Durchlauf erneut versucht)")

    # ------------------------------------------------------------------ member update listener

    @commands.Cog.listener()
    async def on_member_update(self, before: discord.Member, after: discord.Member) -> None:
        """Löst einen Sync mit ~5s Debounce aus, wenn sich Coach-Rollen-Mitgliedschaft ändert."""
        before_roles = {r.id for r in before.roles}
        after_roles = {r.id for r in after.roles}
        if settings.coach_role_id not in (before_roles ^ after_roles):
            return  # Coach-Rolle nicht betroffen

        loop = asyncio.get_event_loop()
        if self._debounce_handle:
            self._debounce_handle.cancel()
        self._debounce_handle = loop.call_later(5.0, self._schedule_role_sync)

    def _schedule_role_sync(self) -> None:
        self._debounce_handle = None
        asyncio.create_task(self._do_role_sync())

    # ------------------------------------------------------------------ notification loop

    async def _notification_loop(self) -> None:
        await self.bot.wait_until_ready()
        while True:
            try:
                await self._process_notifications()
            except Exception as exc:
                log.error("Notification-Loop Fehler: %s", exc)
            await asyncio.sleep(60)

    async def _process_notifications(self) -> None:
        items = await website_client.get_due_notifications()
        if not items:
            return

        to_ack: list[dict] = []

        for item in items:
            user_id = item.get("discord_user_id")
            if not user_id:
                log.warning("Notification ohne discord_user_id – übersprungen: %s", item.get("appointment_id"))
                continue

            text = _build_dm_text(item)
            if text is None:
                continue

            # Empfänger auflösen
            guild = self.bot.guilds[0] if self.bot.guilds else None
            user = guild.get_member(int(user_id)) if guild else None
            if user is None:
                try:
                    user = await self.bot.fetch_user(int(user_id))
                except Exception as exc:
                    log.warning("Nutzer %s konnte nicht aufgelöst werden: %s", user_id, exc)
                    # Nicht acken – nächster Durchlauf versucht es erneut
                    continue

            # DM senden
            try:
                await user.send(text)
                to_ack.append(item)
                log.info("Termin-DM gesendet an %s (Typ: %s)", user_id, item.get("type"))
            except discord.Forbidden:
                # DMs deaktiviert – trotzdem acken, sonst Endlos-Retry
                log.warning(
                    "DMs deaktiviert für Nutzer %s (Typ: %s) – wird geackt ohne Zustellung",
                    user_id,
                    item.get("type"),
                )
                to_ack.append(item)
            except Exception as exc:
                # Netzfehler o.ä. – NICHT acken, beim nächsten Durchlauf erneut versuchen
                log.warning("DM-Versand an %s fehlgeschlagen (wird erneut versucht): %s", user_id, exc)

        if to_ack:
            ok = await website_client.ack_notifications(to_ack)
            if ok:
                log.info("Notifications geackt: %d", len(to_ack))
            else:
                log.warning("Notification-Ack fehlgeschlagen – beim nächsten Durchlauf erneut versucht")


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(CoachingPlatformSyncCog(bot))
