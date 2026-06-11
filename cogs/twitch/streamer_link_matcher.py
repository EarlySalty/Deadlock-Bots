"""Automatischer Discord-Abgleich für Twitch-Streamer.

Problem: Streamer verknüpfen ihren Twitch-Kanal nicht im Dashboard mit Discord,
darum bekommen sie auch nie die Streamer-Rolle. Dieser Cog rät die Verknüpfung
selbst: Er zieht die unverknüpften Streamer vom Twitch-Worker (interne API),
gleicht ihre Logins gegen die Discord-Memberliste ab, lässt eine AI die
Wahrscheinlichkeit „dieselbe Person" bewerten und handelt nach Schwellen:

* Score >= AUTO  -> automatisch verknüpfen + Streamer-Rolle vergeben.
* Score >= REVIEW -> Vorschlag mit Buttons im Operations-Kanal (Admin bestätigt).
* darunter        -> verworfen, aber als „geprüft" vermerkt (kein Re-Scoring).

Rein gescrapte Kanäle (``is_monitored_only``) werden nie automatisch verknüpft,
sondern höchstens vorgeschlagen.

Jede Operation wird in den konfigurierten Discord-Kanal gemeldet, damit der
Betrieb nachvollziehbar bleibt. Der Schreibpfad (discord_user_id setzen) läuft
über den bestehenden internen Endpoint ``/streamers/{login}/discord-profile``;
die Rolle vergibt dieser Cog direkt, weil der Master-Bot den vollen Member-Cache
hat.
"""

from __future__ import annotations

import asyncio
import json
import logging
import os
import re
import secrets
import unicodedata
from datetime import UTC, datetime
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit, urlunsplit

import aiohttp
import discord
from discord.ext import commands, tasks

log = logging.getLogger(__name__)

# --- Konfiguration (env-überschreibbar) --------------------------------------

DEFAULT_NOTIFY_CHANNEL_ID = 1374364800817303632
DEFAULT_STREAMER_ROLE_ID = 1313624729466441769
DEFAULT_GUILD_ID = 1289721245281292288

INTERNAL_API_BASE_PATH = "/internal/twitch/v1"
INTERNAL_TOKEN_HEADER = "X-Internal-Token"  # noqa: S105 — HTTP-Header-Name, kein Secret

STATE_PATH = Path(__file__).resolve().parents[2] / "data" / "streamer_link_state.json"

# Tokens/Affixe, die in Discord-Namen häufig zusätzlich zum eigentlichen Namen
# stehen ("name | ttv", "nameLIVE", ...) und für den Vergleich entfernt werden.
_AFFIXES = {
    "ttv", "live", "twitch", "stream", "streams", "streamer", "yt", "youtube",
    "tv", "official", "real", "the", "its", "im", "iam", "gg",
}
_LEET = {"0": "o", "1": "i", "3": "e", "4": "a", "5": "s", "7": "t", "$": "s", "@": "a", "8": "b"}


def _env_int(name: str, default: int) -> int:
    raw = (os.getenv(name) or "").strip()
    if not raw:
        return default
    try:
        return int(raw)
    except ValueError:
        return default


def _env_float(name: str, default: float) -> float:
    raw = (os.getenv(name) or "").strip()
    if not raw:
        return default
    try:
        return float(raw)
    except ValueError:
        return default


def _env_bool(name: str, default: bool) -> bool:
    raw = (os.getenv(name) or "").strip().lower()
    if not raw:
        return default
    return raw in {"1", "true", "yes", "on"}


# --- Namens-Normalisierung & Fuzzy-Match (pure, testbar) ---------------------

def _deaccent(value: str) -> str:
    return (
        unicodedata.normalize("NFKD", str(value or ""))
        .encode("ascii", "ignore")
        .decode("ascii")
    )


def _tokens(value: str) -> list[str]:
    base = _deaccent(value).lower()
    base = "".join(_LEET.get(ch, ch) for ch in base)
    raw = [t for t in re.split(r"[^a-z0-9]+", base) if t]
    kept = [t for t in raw if t not in _AFFIXES]
    return kept or raw


def norm_key(value: str) -> str:
    """Normalisiert einen Namen zu einem vergleichbaren Schlüssel.

    Akzente/Emoji raus, lower-case, Leetspeak ersetzt, Stream-Affixe wie
    ``ttv``/``live`` entfernt, alles zu reinem [a-z0-9] verdichtet.
    """
    return "".join(_tokens(value))


def similarity(a: str, b: str) -> float:
    """Ähnlichkeit 0..1 zweier bereits normalisierter Schlüssel."""
    if not a or not b:
        return 0.0
    if a == b:
        return 1.0
    from difflib import SequenceMatcher

    return SequenceMatcher(None, a, b).ratio()


def fallback_score(ratio: float, *, exact_unique: bool) -> int:
    """Deterministischer Score, falls die AI nicht verfügbar ist.

    Bewusst konservativ: nur ein eindeutiger, exakter Normalisierungs-Treffer
    erreicht den Auto-Bereich; alles andere bleibt Vorschlag oder fällt raus.
    """
    if exact_unique and ratio >= 0.999:
        return 92
    if ratio >= 0.93:
        return 80
    if ratio >= 0.82:
        return 72
    return int(round(ratio * 70))


def parse_ai_score(text: str | None) -> tuple[int | None, str]:
    """Extrahiert ``{"score":..,"reason":..}`` aus der AI-Antwort.

    Entfernt vorab einen etwaigen ``<think>...</think>``-Block (MiniMax) und
    sucht das erste JSON-Objekt. Gibt (score|None, reason) zurück.
    """
    if not text:
        return None, ""
    cleaned = re.sub(r"<think>.*?</think>", "", text, flags=re.DOTALL | re.IGNORECASE).strip()
    match = re.search(r"\{.*?\}", cleaned, flags=re.DOTALL)
    if not match:
        return None, ""
    try:
        data = json.loads(match.group(0))
    except Exception:
        return None, ""
    raw_score = data.get("score")
    try:
        score = int(round(float(raw_score)))
    except (TypeError, ValueError):
        return None, str(data.get("reason") or "")
    score = max(0, min(100, score))
    return score, str(data.get("reason") or "").strip()[:300]


# --- Persistenter Zustand (verhindert Re-Scoring & hält offene Reviews) -------

class _LinkState:
    """Kleiner JSON-Store: bewertete Logins + offene Button-Vorschläge."""

    def __init__(self, path: Path) -> None:
        self._path = path
        self._lock = asyncio.Lock()
        self.processed: dict[str, dict[str, Any]] = {}
        self.pending: dict[str, dict[str, Any]] = {}
        self.manual_pending: dict[str, dict[str, Any]] = {}

    def load(self) -> None:
        try:
            data = json.loads(self._path.read_text(encoding="utf-8"))
            self.processed = dict(data.get("processed") or {})
            self.pending = dict(data.get("pending") or {})
            self.manual_pending = dict(data.get("manual_pending") or {})
        except FileNotFoundError:
            self.processed, self.pending, self.manual_pending = {}, {}, {}
        except Exception:
            log.exception("streamer_link state load failed; starting empty")
            self.processed, self.pending, self.manual_pending = {}, {}, {}

    async def save(self) -> None:
        async with self._lock:
            await asyncio.to_thread(self._save_sync)

    def _save_sync(self) -> None:
        self._path.parent.mkdir(parents=True, exist_ok=True)
        tmp = self._path.with_suffix(".tmp")
        tmp.write_text(
            json.dumps(
                {"processed": self.processed, "pending": self.pending, "manual_pending": self.manual_pending},
                ensure_ascii=False, indent=0,
            ),
            encoding="utf-8",
        )
        tmp.replace(self._path)

    def is_handled(self, login: str) -> bool:
        login = login.lower()
        if login in self.processed:
            return True
        return any(p.get("login") == login for p in self.pending.values())

    def mark(self, login: str, status: str, **extra: Any) -> None:
        self.processed[login.lower()] = {
            "status": status,
            "at": datetime.now(UTC).isoformat(),
            **extra,
        }


# --- Interner API-Client (Token-gegated, Loopback) ---------------------------

def _resolve_internal_base_url() -> str | None:
    base = (os.getenv("TWITCH_INTERNAL_API_BASE_URL") or "").strip()
    if not base:
        host = (os.getenv("TWITCH_INTERNAL_API_HOST") or "127.0.0.1").strip() or "127.0.0.1"
        port = _env_int("TWITCH_INTERNAL_API_PORT", 8776)
        base = f"http://{host}:{port}"
    if "://" not in base:
        base = f"http://{base}"
    parsed = urlsplit(base)
    path = (parsed.path or "").rstrip("/")
    if path.endswith(INTERNAL_API_BASE_PATH.rstrip("/")):
        path = path[: -len(INTERNAL_API_BASE_PATH.rstrip("/"))]
    return urlunsplit((parsed.scheme or "http", parsed.netloc, path.rstrip("/"), "", ""))


class _InternalApiClient:
    def __init__(self, base_url: str, token: str, *, timeout: float = 10.0) -> None:
        self._base = base_url.rstrip("/")
        self._token = token
        self._timeout = timeout
        self._session: aiohttp.ClientSession | None = None

    @classmethod
    def from_env(cls) -> _InternalApiClient | None:
        token = (os.getenv("TWITCH_INTERNAL_API_TOKEN") or "").strip()
        base = _resolve_internal_base_url()
        if not token or not base:
            return None
        return cls(base, token, timeout=_env_float("TWITCH_INTERNAL_API_TIMEOUT_SEC", 10.0))

    async def _ensure(self) -> aiohttp.ClientSession:
        if self._session is None or self._session.closed:
            self._session = aiohttp.ClientSession(
                timeout=aiohttp.ClientTimeout(total=self._timeout)
            )
        return self._session

    async def close(self) -> None:
        if self._session is not None and not self._session.closed:
            await self._session.close()

    async def _request(self, method: str, path: str, payload: dict | None = None) -> Any:
        session = await self._ensure()
        url = f"{self._base}{path}"
        headers = {INTERNAL_TOKEN_HEADER: self._token}
        async with session.request(method, url, json=payload, headers=headers) as resp:
            text = await resp.text()
            if resp.status >= 400:
                raise RuntimeError(f"internal api {method} {path} -> {resp.status}: {text[:200]}")
            return json.loads(text) if text else {}

    async def list_unlinked(self) -> list[dict[str, Any]]:
        data = await self._request("GET", f"{INTERNAL_API_BASE_PATH}/streamers/link-candidates")
        entries = data.get("entries") if isinstance(data, dict) else None
        return [e for e in (entries or []) if isinstance(e, dict)]

    async def link_discord_profile(
        self, login: str, *, discord_user_id: str, discord_display_name: str
    ) -> dict[str, Any]:
        path = f"{INTERNAL_API_BASE_PATH}/streamers/{login.lower()}/discord-profile"
        return await self._request(
            "POST",
            path,
            {
                "discord_user_id": str(discord_user_id),
                "discord_display_name": discord_display_name,
                "mark_member": True,
            },
        )


# --- Button-View für Review-Vorschläge (persistent über Neustart) ------------

class PendingLinkView(discord.ui.View):
    def __init__(self, cog: StreamerLinkMatcher, token: str) -> None:
        super().__init__(timeout=None)
        self.cog = cog
        self.token = token
        link_btn = discord.ui.Button(
            label="Verknüpfen", style=discord.ButtonStyle.success, custom_id=f"slm:link:{token}"
        )
        reject_btn = discord.ui.Button(
            label="Ablehnen", style=discord.ButtonStyle.danger, custom_id=f"slm:reject:{token}"
        )
        link_btn.callback = self._on_link
        reject_btn.callback = self._on_reject
        self.add_item(link_btn)
        self.add_item(reject_btn)

    async def _guard(self, interaction: discord.Interaction) -> bool:
        perms = getattr(interaction.user, "guild_permissions", None)
        if perms is None or not (perms.manage_roles or perms.administrator):
            await interaction.response.send_message(
                "Nur Mods mit Rollen-Rechten können das bestätigen.", ephemeral=True
            )
            return False
        return True

    async def _on_link(self, interaction: discord.Interaction) -> None:
        if not await self._guard(interaction):
            return
        await self.cog.confirm_pending(interaction, self.token, approve=True)

    async def _on_reject(self, interaction: discord.Interaction) -> None:
        if not await self._guard(interaction):
            return
        await self.cog.confirm_pending(interaction, self.token, approve=False)


# --- Manueller Link-Dialog (kein Auto-Match) ----------------------------------

class ManualLinkModal(discord.ui.Modal, title="Discord-Account verknüpfen"):
    discord_input = discord.ui.TextInput(
        label="Discord-Name oder ID",
        placeholder="z.B. username oder 123456789012345678",
        min_length=2,
        max_length=100,
    )

    def __init__(self, cog: "StreamerLinkMatcher", login: str) -> None:
        super().__init__()
        self.cog = cog
        self.login = login

    async def on_submit(self, interaction: discord.Interaction) -> None:
        value = self.discord_input.value.strip()
        guild = self.cog._guild()
        if guild is None:
            await interaction.response.send_message("Guild nicht gefunden.", ephemeral=True)
            return

        member: discord.Member | None = None
        if value.isdigit():
            member = guild.get_member(int(value))
        if member is None:
            vl = value.lower()
            for m in guild.members:
                if m.bot:
                    continue
                if (
                    m.name.lower() == vl
                    or (m.global_name or "").lower() == vl
                    or (m.nick or "").lower() == vl
                ):
                    member = m
                    break

        if member is None:
            await interaction.response.send_message(
                f"Kein Member für `{value}` gefunden. Bitte numerische Discord-ID eingeben.",
                ephemeral=True,
            )
            return

        display = member.global_name or member.name
        try:
            await self.cog._client.link_discord_profile(
                self.login, discord_user_id=str(member.id), discord_display_name=display
            )
        except Exception as exc:
            await interaction.response.send_message(
                f"DB-Write fehlgeschlagen: `{str(exc)[:160]}`", ephemeral=True
            )
            return

        role_note = await self.cog._grant_role(guild, member)
        self.cog.state.processed.pop(self.login.lower(), None)
        self.cog.state.mark(self.login, "linked", discord_user_id=str(member.id), by=str(interaction.user))
        self.cog.state.manual_pending.pop(self.login.lower(), None)
        await self.cog.state.save()

        await interaction.response.send_message(
            f"✅ **{self.login}** → {member.mention} verknüpft. {role_note}"
        )
        try:
            if interaction.message:
                embed = interaction.message.embeds[0] if interaction.message.embeds else discord.Embed()
                embed.color = discord.Color(0x2ECC71)
                embed.add_field(
                    name="Status",
                    value=f"Manuell verknüpft von {interaction.user.mention} → {member.mention}. {role_note}",
                    inline=False,
                )
                await interaction.message.edit(embed=embed, view=None)
        except Exception:
            log.debug("Konnte Manual-Link-Message nicht finalisieren", exc_info=True)


class ManualLinkView(discord.ui.View):
    def __init__(self, cog: "StreamerLinkMatcher", login: str) -> None:
        super().__init__(timeout=None)
        self.cog = cog
        self.login = login
        btn = discord.ui.Button(
            label="Discord eingeben",
            style=discord.ButtonStyle.primary,
            custom_id=f"slm:manual:{login}",
        )
        btn.callback = self._on_click
        self.add_item(btn)

    async def _on_click(self, interaction: discord.Interaction) -> None:
        perms = getattr(interaction.user, "guild_permissions", None)
        if perms is None or not (perms.manage_roles or perms.administrator):
            await interaction.response.send_message("Nur Mods mit Rollen-Rechten.", ephemeral=True)
            return
        await interaction.response.send_modal(ManualLinkModal(self.cog, self.login))


# --- Cog ---------------------------------------------------------------------

class StreamerLinkMatcher(commands.Cog):
    """Backfill + laufender Abgleich neuer Streamer gegen die Memberliste."""

    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot
        self.enabled = _env_bool("STREAMER_LINK_ENABLED", True)
        self.notify_channel_id = _env_int("STREAMER_LINK_NOTIFY_CHANNEL_ID", DEFAULT_NOTIFY_CHANNEL_ID)
        self.role_id = _env_int("STREAMER_ROLE_ID", DEFAULT_STREAMER_ROLE_ID)
        self.guild_id = _env_int("STREAMER_GUILD_ID", 0) or _env_int("MAIN_GUILD_ID", 0) or DEFAULT_GUILD_ID
        self.auto_threshold = _env_int("STREAMER_LINK_AUTO_THRESHOLD", 90)
        self.review_threshold = _env_int("STREAMER_LINK_REVIEW_THRESHOLD", 70)
        self.fuzzy_floor = _env_float("STREAMER_LINK_FUZZY_FLOOR", 0.62)
        self.max_ai_per_scan = _env_int("STREAMER_LINK_MAX_AI_PER_SCAN", 40)
        self.scan_interval_hours = _env_int("STREAMER_LINK_SCAN_INTERVAL_HOURS", 6)
        self.ai_provider = (os.getenv("STREAMER_LINK_AI_PROVIDER") or "minimax").strip().lower()
        self.state = _LinkState(STATE_PATH)
        self._client: _InternalApiClient | None = None
        self._scan_lock = asyncio.Lock()
        self._primed = False  # erste Loop-Iteration (sofort beim Start) überspringen

    async def cog_load(self) -> None:
        self.state.load()
        self._client = _InternalApiClient.from_env()
        if self._client is None:
            log.warning("StreamerLinkMatcher: kein interner API-Token/URL – Cog inaktiv.")
            self.enabled = False
            return
        # Offene Review-Buttons nach Neustart wieder aktiv schalten.
        for token, rec in list(self.state.pending.items()):
            message_id = rec.get("message_id")
            if message_id:
                try:
                    self.bot.add_view(PendingLinkView(self, token), message_id=int(message_id))
                except Exception:
                    log.debug("Konnte Review-View %s nicht reaktivieren", token, exc_info=True)
        for login, rec in list(self.state.manual_pending.items()):
            message_id = rec.get("message_id")
            if message_id:
                try:
                    self.bot.add_view(ManualLinkView(self, login), message_id=int(message_id))
                except Exception:
                    log.debug("Konnte Manual-Link-View %s nicht reaktivieren", login, exc_info=True)
        if self.enabled and self.scan_interval_hours > 0:
            self.incremental_scan.change_interval(hours=self.scan_interval_hours)
            self.incremental_scan.start()

    async def cog_unload(self) -> None:
        self.incremental_scan.cancel()
        if self._client is not None:
            await self._client.close()

    # ---- Discord-Helfer ----

    def _guild(self) -> discord.Guild | None:
        guild = self.bot.get_guild(self.guild_id)
        if guild is not None:
            return guild
        # Fallback: Guild über die Rolle suchen.
        for g in self.bot.guilds:
            if g.get_role(self.role_id) is not None:
                return g
        return self.bot.guilds[0] if self.bot.guilds else None

    def _notify_channel(self) -> discord.abc.Messageable | None:
        ch = self.bot.get_channel(self.notify_channel_id)
        return ch if isinstance(ch, discord.abc.Messageable) else None

    async def _notify(self, embed: discord.Embed, view: discord.ui.View | None = None) -> discord.Message | None:
        ch = self._notify_channel()
        if ch is None:
            log.warning("StreamerLinkMatcher: Notify-Kanal %s nicht gefunden", self.notify_channel_id)
            return None
        try:
            return await ch.send(embed=embed, view=view)
        except Exception:
            log.exception("StreamerLinkMatcher: konnte Notify nicht senden")
            return None

    # ---- AI-Bewertung ----

    async def _ai_score(self, login: str, member: discord.Member, ratio: float) -> tuple[int | None, str]:
        ai = self.bot.get_cog("AIConnector")
        if ai is None or not hasattr(ai, "generate_text"):
            return None, ""
        system = (
            "Du bewertest, ob ein Discord-Nutzer und ein Twitch-Streamer dieselbe Person sind, "
            "ausschließlich anhand der Namen. Antworte NUR mit JSON: "
            '{"score": <0-100>, "reason": "<kurze Begründung>"}. '
            "score ist die Wahrscheinlichkeit in Prozent. Kein weiterer Text."
        )
        prompt = (
            f"Twitch-Login: {login}\n"
            f"Discord-Username: {member.name}\n"
            f"Discord-Anzeigename: {member.global_name or '-'}\n"
            f"Discord-Servername: {member.nick or '-'}\n"
            f"String-Ähnlichkeit der normalisierten Namen (0-1): {ratio:.2f}\n"
            "Wie wahrscheinlich ist es dieselbe Person?"
        )
        try:
            text, _meta = await ai.generate_text(
                provider=self.ai_provider,
                prompt=prompt,
                system_prompt=system,
                max_output_tokens=160,
                temperature=0.0,
            )
        except Exception:
            log.exception("StreamerLinkMatcher: AI-Score fehlgeschlagen für %s", login)
            return None, ""
        return parse_ai_score(text)

    # ---- Matching ----

    def _build_member_index(self, guild: discord.Guild) -> tuple[dict[str, list[discord.Member]], dict[str, list[tuple[str, discord.Member]]]]:
        exact: dict[str, list[discord.Member]] = {}
        bucket: dict[str, list[tuple[str, discord.Member]]] = {}
        for member in guild.members:
            if member.bot:
                continue
            keys = {
                norm_key(member.name),
                norm_key(member.global_name or ""),
                norm_key(member.nick or ""),
            }
            for key in keys:
                if not key:
                    continue
                exact.setdefault(key, []).append(member)
                bucket.setdefault(key[:2], []).append((key, member))
        return exact, bucket

    def _best_member(
        self,
        login_key: str,
        exact: dict[str, list[discord.Member]],
        bucket: dict[str, list[tuple[str, discord.Member]]],
    ) -> tuple[discord.Member | None, float, bool]:
        if login_key in exact:
            members = exact[login_key]
            return members[0], 1.0, len(members) == 1
        best_member: discord.Member | None = None
        best_ratio = 0.0
        for key, member in bucket.get(login_key[:2], []):
            ratio = similarity(login_key, key)
            if ratio > best_ratio:
                best_ratio, best_member = ratio, member
        return best_member, best_ratio, False

    async def _run_scan(self, *, trigger: str) -> dict[str, Any]:
        stats: dict[str, Any] = {"checked": 0, "auto": 0, "review": 0, "skipped": 0, "errors": 0, "ai_calls": 0, "new_logins": []}
        if not self.enabled or self._client is None:
            return stats
        guild = self._guild()
        if guild is None:
            await self._notify(self._error_embed("Keine Guild gefunden – Abgleich abgebrochen."))
            return stats
        if not guild.members or len(guild.members) < 2:
            try:
                await guild.chunk()
            except Exception:
                log.debug("guild.chunk fehlgeschlagen", exc_info=True)

        try:
            candidates = await self._client.list_unlinked()
        except Exception:
            log.exception("StreamerLinkMatcher: Kandidaten-Abruf fehlgeschlagen")
            await self._notify(self._error_embed("Konnte unverknüpfte Streamer nicht laden (interne API)."))
            stats["errors"] += 1
            return stats

        exact, bucket = self._build_member_index(guild)
        used_member_ids: set[int] = set()
        ai_available = self.bot.get_cog("AIConnector") is not None

        for entry in candidates:
            login = str(entry.get("twitch_login") or "").strip().lower()
            if not login:
                continue
            # Bereits bewertet/offen -> nie erneut scoren (Backfill einmal, dann nur Neue).
            if self.state.is_handled(login):
                continue
            stats["checked"] += 1
            stats["new_logins"].append(login)
            is_monitored = bool(entry.get("is_monitored_only"))
            login_key = norm_key(login)
            if not login_key:
                self.state.mark(login, "no_match", reason="leerer Schlüssel")
                stats["skipped"] += 1
                continue

            member, ratio, exact_unique = self._best_member(login_key, exact, bucket)
            if member is None or ratio < self.fuzzy_floor:
                self.state.mark(login, "no_match", reason=f"kein Member (beste Ähnlichkeit {ratio:.2f})")
                stats["skipped"] += 1
                await self._post_manual_link_prompt(login)
                continue
            if member.id in used_member_ids:
                # Member schon in diesem Lauf einem anderen Streamer zugeordnet.
                self.state.mark(login, "no_match", reason="Member-Kollision im Lauf")
                stats["skipped"] += 1
                continue

            # AI-Budget pro Lauf gedeckelt. Ist es erschöpft, brechen wir ab und
            # lassen den Rest unbewertet -- der nächste Lauf macht mit frischem
            # Budget weiter (Backfill bleibt über mehrere Läufe fortsetzbar).
            if ai_available and stats["ai_calls"] >= self.max_ai_per_scan:
                stats["checked"] -= 1
                break

            score, reason = (None, "")
            if ai_available:
                score, reason = await self._ai_score(login, member, ratio)
                if score is not None:
                    stats["ai_calls"] += 1
            if score is None:
                score = fallback_score(ratio, exact_unique=exact_unique)
                reason = reason or f"Heuristik (Ähnlichkeit {ratio:.2f})"

            await asyncio.sleep(0)  # Event-Loop atmen lassen

            can_auto = score >= self.auto_threshold and not is_monitored
            if can_auto:
                ok = await self._auto_link(guild, login, entry, member, score, reason)
                if ok:
                    used_member_ids.add(member.id)
                    stats["auto"] += 1
                else:
                    stats["errors"] += 1
            elif score >= self.review_threshold:
                await self._post_review(login, entry, member, score, reason, monitored=is_monitored)
                used_member_ids.add(member.id)
                stats["review"] += 1
            else:
                self.state.mark(login, "no_match", reason=f"Score {score} < {self.review_threshold}")
                stats["skipped"] += 1
                await self._post_manual_link_prompt(login)

        await self.state.save()
        await self._notify(self._summary_embed(stats, trigger))
        return stats

    async def _auto_link(
        self,
        guild: discord.Guild,
        login: str,
        entry: dict[str, Any],
        member: discord.Member,
        score: int,
        reason: str,
    ) -> bool:
        display = member.global_name or member.name
        try:
            await self._client.link_discord_profile(
                login, discord_user_id=str(member.id), discord_display_name=display
            )
        except Exception as exc:
            log.exception("StreamerLinkMatcher: Auto-Link DB-Write fehlgeschlagen für %s", login)
            await self._notify(
                self._error_embed(f"Auto-Link für **{login}** → {member.mention} fehlgeschlagen: `{str(exc)[:160]}`")
            )
            return False
        role_note = await self._grant_role(guild, member)
        self.state.mark(login, "auto_linked", discord_user_id=str(member.id), score=score)
        embed = discord.Embed(
            title="✅ Auto-verknüpft",
            description=(
                f"**Twitch:** `{login}`\n"
                f"**Discord:** {member.mention} (`{member.name}`)\n"
                f"**Wahrscheinlichkeit:** {score}%\n"
                f"**Grund:** {reason}\n"
                f"{role_note}"
            ),
            color=0x2ECC71,
        )
        await self._notify(embed)
        return True

    async def _grant_role(self, guild: discord.Guild, member: discord.Member) -> str:
        role = guild.get_role(self.role_id)
        if role is None:
            return "⚠️ Streamer-Rolle nicht gefunden – nur verknüpft."
        if role in member.roles:
            return "Streamer-Rolle war bereits vergeben."
        try:
            await member.add_roles(role, reason="Auto-Match Twitch↔Discord")
            return "Streamer-Rolle vergeben."
        except discord.Forbidden:
            return "⚠️ Keine Berechtigung für die Streamer-Rolle."
        except Exception:
            log.exception("StreamerLinkMatcher: add_roles fehlgeschlagen")
            return "⚠️ Rolle konnte nicht vergeben werden."

    async def _post_review(
        self,
        login: str,
        entry: dict[str, Any],
        member: discord.Member,
        score: int,
        reason: str,
        *,
        monitored: bool,
    ) -> None:
        token = secrets.token_hex(8)
        note = "\n*(nur überwachter Kanal – nie automatisch)*" if monitored else ""
        embed = discord.Embed(
            title="❓ Möglicher Streamer-Match",
            description=(
                f"**Twitch:** `{login}`\n"
                f"**Discord:** {member.mention} (`{member.name}`)\n"
                f"**Wahrscheinlichkeit:** {score}%\n"
                f"**Grund:** {reason}{note}"
            ),
            color=0xF1C40F,
        )
        view = PendingLinkView(self, token)
        msg = await self._notify(embed, view=view)
        self.state.pending[token] = {
            "login": login,
            "twitch_user_id": str(entry.get("twitch_user_id") or ""),
            "discord_user_id": str(member.id),
            "discord_display_name": member.global_name or member.name,
            "score": score,
            "reason": reason,
            "message_id": msg.id if msg else None,
            "channel_id": msg.channel.id if msg else None,
        }

    async def confirm_pending(self, interaction: discord.Interaction, token: str, *, approve: bool) -> None:
        rec = self.state.pending.get(token)
        if rec is None:
            await interaction.response.send_message("Dieser Vorschlag ist nicht mehr offen.", ephemeral=True)
            return
        await interaction.response.defer()
        login = str(rec.get("login") or "")
        if not approve:
            self.state.pending.pop(token, None)
            self.state.mark(login, "rejected", by=str(interaction.user))
            await self.state.save()
            await self._finalize_message(interaction, f"❌ Abgelehnt von {interaction.user.mention} – **{login}**", 0x95A5A6)
            return

        guild = self._guild()
        member = guild.get_member(int(rec["discord_user_id"])) if guild else None
        if guild is None or member is None:
            await interaction.followup.send("Member oder Guild nicht gefunden.", ephemeral=True)
            return
        try:
            await self._client.link_discord_profile(
                login, discord_user_id=str(member.id), discord_display_name=str(rec.get("discord_display_name") or member.name)
            )
        except Exception as exc:
            await interaction.followup.send(f"DB-Write fehlgeschlagen: `{str(exc)[:160]}`", ephemeral=True)
            return
        role_note = await self._grant_role(guild, member)
        self.state.pending.pop(token, None)
        self.state.mark(login, "linked", discord_user_id=str(member.id), by=str(interaction.user))
        await self.state.save()
        await self._finalize_message(
            interaction,
            f"✅ Bestätigt von {interaction.user.mention} – **{login}** → {member.mention}. {role_note}",
            0x2ECC71,
        )

    async def _finalize_message(self, interaction: discord.Interaction, text: str, color: int) -> None:
        try:
            embed = interaction.message.embeds[0] if interaction.message and interaction.message.embeds else discord.Embed()
            embed.color = discord.Color(color)
            embed.add_field(name="Status", value=text, inline=False)
            await interaction.message.edit(embed=embed, view=None)
        except Exception:
            log.debug("Konnte Review-Nachricht nicht finalisieren", exc_info=True)

    async def _post_manual_link_prompt(self, login: str) -> None:
        embed = discord.Embed(
            title="🔗 Kein Discord-Match",
            description=(
                f"**Twitch:** `{login}`\n"
                "Kein Discord-Account automatisch gefunden.\n"
                "Discord-Name oder numerische ID eingeben, um manuell zu verknüpfen."
            ),
            color=0xE67E22,
        )
        view = ManualLinkView(self, login)
        msg = await self._notify(embed, view=view)
        self.state.manual_pending[login.lower()] = {
            "login": login,
            "message_id": msg.id if msg else None,
            "channel_id": msg.channel.id if msg else None,
        }

    # ---- Embeds ----

    def _summary_embed(self, stats: dict[str, Any], trigger: str) -> discord.Embed:
        logins: list[str] = stats.get("new_logins") or []
        logins_text = ""
        if logins:
            shown = logins[:10]
            rest = len(logins) - len(shown)
            names = ", ".join(f"`{l}`" for l in shown)
            logins_text = f"\n**Neu:** {names}"
            if rest > 0:
                logins_text += f" +{rest} weitere"
        return discord.Embed(
            title="📊 Streamer-Abgleich gelaufen",
            description=(
                f"**Auslöser:** {trigger}\n"
                f"**Geprüft:** {stats['checked']}\n"
                f"**Auto-verknüpft:** {stats['auto']}\n"
                f"**Vorschläge:** {stats['review']}\n"
                f"**Ohne Treffer:** {stats['skipped']}\n"
                f"**AI-Aufrufe:** {stats['ai_calls']}\n"
                f"**Fehler:** {stats['errors']}"
                f"{logins_text}"
            ),
            color=0x3498DB,
        )

    def _error_embed(self, text: str) -> discord.Embed:
        return discord.Embed(title="⚠️ Streamer-Abgleich", description=text, color=0xE74C3C)

    # ---- Trigger ----

    @tasks.loop(hours=6)
    async def incremental_scan(self) -> None:
        # Die erste Iteration feuert sofort beim Start -- überspringen, damit ein
        # Neustart keinen ungefragten Voll-Scan auslöst. Den initialen Backfill
        # startet ein Admin bewusst per !twitch_link_scan.
        if not self._primed:
            self._primed = True
            return
        async with self._scan_lock:
            await self._run_scan(trigger="Auto-Scan (neue Streamer)")

    @incremental_scan.before_loop
    async def _before_incremental(self) -> None:
        await self.bot.wait_until_ready()

    @commands.command(name="twitch_link_scan")
    @commands.has_permissions(administrator=True)
    async def twitch_link_scan(self, ctx: commands.Context) -> None:
        """Voller Backfill-Abgleich aller unverknüpften Streamer (Admin)."""
        if not self.enabled:
            await ctx.reply("Matcher ist inaktiv (kein interner API-Token?).")
            return
        if self._scan_lock.locked():
            await ctx.reply("Es läuft bereits ein Abgleich.")
            return
        await ctx.reply("Starte vollständigen Streamer-Abgleich … Ergebnisse landen im Ops-Kanal.")
        async with self._scan_lock:
            stats = await self._run_scan(trigger=f"Manuell ({ctx.author})")
        await ctx.reply(
            f"Fertig: {stats['auto']} auto, {stats['review']} Vorschläge, "
            f"{stats['skipped']} ohne Treffer, {stats['errors']} Fehler."
        )

    @commands.command(name="twitch_link_rescan_login")
    @commands.has_permissions(administrator=True)
    async def twitch_link_rescan_login(self, ctx: commands.Context, login: str) -> None:
        """Einen einzelnen Login erneut zur Bewertung freigeben (Admin)."""
        key = login.strip().lower()
        self.state.processed.pop(key, None)
        for token, rec in list(self.state.pending.items()):
            if rec.get("login") == key:
                self.state.pending.pop(token, None)
        await self.state.save()
        await ctx.reply(f"`{key}` ist wieder offen für den nächsten Abgleich.")


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(StreamerLinkMatcher(bot))
