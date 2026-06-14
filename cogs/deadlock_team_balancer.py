# ============================================================
# Deadlock Team Balancer – volle Parität, neue DB (aiosqlite)
# Datei: cogs/deadlock_team_balancer.py
# Erfordert: utils.deadlock_db.DB_PATH  (SQLite, Tabelle: user_ranks(user_id INTEGER PRIMARY KEY, rank TEXT))
# Kategorie-ID (Pflicht, 2 Team-VCs): 1289721245281292290
# ============================================================

import asyncio
import logging
import random
from dataclasses import dataclass
from datetime import datetime, timedelta
from functools import lru_cache
from itertools import combinations

import discord
from discord.ext import commands

from cogs.customgames import tournament_store as tstore
from service import db

logger = logging.getLogger(__name__)

# --------- KONFIG ---------
MATCH_CATEGORY_ID = 1289721245281292290  # Pflichtkategorie für die zwei Team-VCs
TEAM_SIZE_CAP = 6  # max 6 pro Team => 6v6
MOVE_SLEEP = 0.35  # Rate-Limit-Schoner beim Move
SELECTION_TIMEOUT = 90.0  # Sekunden für interaktive Auswahl
TOURNAMENT_UI_TIMEOUT = 240.0  # Timeout fuer Turnier-Menues
# --------------------------

# Rang-Mapping (Name -> Wert)
DEADLOCK_RANKS: dict[str, int] = {
    "Obscurus": 0,
    "Initiate": 1,
    "Seeker": 2,
    "Alchemist": 3,
    "Arcanist": 4,
    "Ritualist": 5,
    "Emissary": 6,
    "Archon": 7,
    "Oracle": 8,
    "Phantom": 9,
    "Ascendant": 10,
    "Eternus": 11,
}

# Rollen-ID -> (RankName, RankValue)
DISCORD_RANK_ROLES: dict[int, tuple[str, int]] = {
    1331457571118387210: ("Initiate", 1),
    1331457652877955072: ("Seeker", 2),
    1331457699992436829: ("Alchemist", 3),
    1331457724848017539: ("Arcanist", 4),
    1331457879345070110: ("Ritualist", 5),
    1331457898781474836: ("Emissary", 6),
    1331457949654319114: ("Archon", 7),
    1316966867033653338: ("Oracle", 8),
    1331458016356208680: ("Phantom", 9),
    1331458049637875785: ("Ascendant", 10),
    1331458087349129296: ("Eternus", 11),
}

# ================= Helper & DB =================


def _normalize_rank_name(raw: str) -> str:
    return (raw or "").strip().title()


@lru_cache(maxsize=10000)
def _rank_value_from_name_cached(name: str) -> int:
    return DEADLOCK_RANKS.get(_normalize_rank_name(name), 0)


async def _fetch_rank_from_db(user_id: int) -> tuple[str, int]:
    """Fetch user rank from central DB using async wrapper."""
    try:
        row = await db.query_one_async(
            "SELECT rank FROM user_ranks WHERE user_id = ?",
            (int(user_id),),
        )
        if row and row[0]:
            nm = _normalize_rank_name(str(row[0]))
            return nm, _rank_value_from_name_cached(nm)
    except Exception as e:
        logger.warning(f"DB rank fetch failed for {user_id}: {e}")
    return "Obscurus", 0


def _rank_from_roles(member: discord.Member) -> tuple[str, int]:
    best = ("Obscurus", 0)
    for role in member.roles:
        meta = DISCORD_RANK_ROLES.get(role.id)
        if meta and meta[1] > best[1]:
            best = meta
    return best


def _balance_score(team_a: list[int], team_b: list[int]) -> float:
    # Kombiniert Summe-/Ø-Differenz + Varianz – je kleiner desto besser
    if not team_a or not team_b:
        return float("inf")
    avg_a = sum(team_a) / len(team_a)
    avg_b = sum(team_b) / len(team_b)
    var_a = sum((x - avg_a) ** 2 for x in team_a) / len(team_a)
    var_b = sum((x - avg_b) ** 2 for x in team_b) / len(team_b)
    diff_sum = abs(sum(team_a) - sum(team_b))
    diff_avg = abs(avg_a - avg_b)
    return diff_sum * 1.0 + diff_avg * 2.0 + (var_a + var_b) * 0.5


def _best_split(
    players: list[tuple[discord.Member, int]],
) -> tuple[list[tuple[discord.Member, int]], list[tuple[discord.Member, int]]]:
    """
    players: [(member, rank_value)]
    -> zwei Teams (gleiche Größe, max TEAM_SIZE_CAP) mit bester Balance
    """
    n = len(players)
    team_size = min(TEAM_SIZE_CAP, n // 2)
    team_size = max(2, team_size)  # Safety

    best = None
    best_score = float("inf")
    idx = list(range(n))

    for comb in combinations(idx, team_size):
        a_idx = set(comb)
        team_a = [players[i] for i in a_idx]
        rest = [players[i] for i in idx if i not in a_idx]
        if len(rest) < team_size:
            continue
        team_b = rest[:team_size]
        score = _balance_score([r for _, r in team_a], [r for _, r in team_b])
        if score < best_score:
            best_score = score
            best = (team_a, team_b)

    return best if best else (players[:team_size], players[team_size : team_size * 2])


def _team_embed(
    team_a: list[tuple[discord.Member, int]],
    team_b: list[tuple[discord.Member, int]],
    title: str,
) -> discord.Embed:
    def fmt(team: list[tuple[discord.Member, int]]) -> tuple[str, float, float]:
        vals = [v for _, v in team]
        avg = sum(vals) / len(vals) if vals else 0.0
        var = (sum((x - avg) ** 2 for x in vals) / len(vals)) if vals else 0.0
        lines = []
        for m, v in team:
            nm = next((k for k, val in DEADLOCK_RANKS.items() if val == v), "Obscurus")
            lines.append(f"• **{m.display_name}** — {nm} ({v})")
        return "\n".join(lines) if lines else "—", avg, var

    emb = discord.Embed(title=title, color=0x00CC88)
    a_txt, a_avg, a_var = fmt(team_a)
    b_txt, b_avg, b_var = fmt(team_b)

    emb.add_field(name=f"🟠 Team Amber (Ø {a_avg:.1f})", value=a_txt, inline=True)
    emb.add_field(name=f"🔵 Team Sapphire (Ø {b_avg:.1f})", value=b_txt, inline=True)

    diff = abs(a_avg - b_avg)
    mark = "✅" if diff < 1.0 else ("⚠️" if diff < 2.0 else "❌")
    emb.add_field(
        name="📊 Balance",
        value=f"{mark} Ø-Unterschied: **{diff:.2f}**\nVarianz A: {a_var:.2f} | Varianz B: {b_var:.2f}",
        inline=False,
    )
    return emb


# ================= Auswahl-UI (>12 Spieler) =================


class PlayerButton(discord.ui.Button):
    def __init__(self, member: discord.Member, rank_val: int, idx: int):
        super().__init__(
            label=f"{member.display_name}",
            style=discord.ButtonStyle.secondary,
            row=(idx % 20) // 5,  # 5 Buttons je Reihe (0..3), Reihe 4 = Control
        )
        self.member = member
        self.rank_val = rank_val
        self.idx = idx

    def set_selected(self, selected: bool):
        self.style = discord.ButtonStyle.success if selected else discord.ButtonStyle.secondary

    async def callback(self, interaction: discord.Interaction):
        view: SelectionView = self.view  # type: ignore
        if self.idx in view.selected:
            view.selected.remove(self.idx)
            self.set_selected(False)
        else:
            if len(view.selected) >= view.max_players:
                await interaction.response.send_message(
                    f"Maximale Auswahl erreicht ({view.max_players}).", ephemeral=True
                )
                return
            view.selected.add(self.idx)
            self.set_selected(True)
        view.refresh_confirm()
        await interaction.response.edit_message(view=view)


class SelectionView(discord.ui.View):
    MAX_COMPONENTS = 25
    CONTROL_COUNT = 2  # confirm + random

    def __init__(
        self,
        ctx: commands.Context,
        players: list[tuple[discord.Member, str, int]],
        max_players: int = 12,
    ):
        super().__init__(timeout=SELECTION_TIMEOUT)
        self.ctx = ctx
        self.players = players
        self.max_players = max_players
        self.selected: set[int] = set()

        allowed = min(len(players), self.MAX_COMPONENTS - self.CONTROL_COUNT)  # 23 Buttons
        for i, (m, _nm, rv) in enumerate(players[:allowed]):
            btn = PlayerButton(m, rv, i)
            self.add_item(btn)

        self.confirm_btn = discord.ui.Button(
            label=f"Match starten (0/{self.max_players})",
            style=discord.ButtonStyle.success,
            emoji="🎮",
            row=4,
            disabled=True,
        )
        self.confirm_btn.callback = self._confirm_cb
        self.add_item(self.confirm_btn)

        self.random_btn = discord.ui.Button(
            label="Zufällige Auswahl",
            style=discord.ButtonStyle.secondary,
            emoji="🎲",
            row=4,
        )
        self.random_btn.callback = self._random_cb
        self.add_item(self.random_btn)

    def refresh_confirm(self):
        c = len(self.selected)
        self.confirm_btn.label = f"Match starten ({c}/{self.max_players})"
        self.confirm_btn.disabled = c < 4 or c > self.max_players

    async def _confirm_cb(self, interaction: discord.Interaction):
        chosen = [self.players[i] for i in sorted(self.selected) if i < len(self.players)]
        self.stop()
        await interaction.response.edit_message(
            content=f"🎮 Starte Match mit {len(chosen)} ausgewählten Spielern …",
            view=None,
            embed=None,
        )
        cog: DeadlockTeamBalancer = interaction.client.get_cog("DeadlockTeamBalancer")  # type: ignore
        if not cog:
            return
        await cog._run_balance_and_start(self.ctx, chosen)

    async def _random_cb(self, interaction: discord.Interaction):
        self.selected.clear()
        count = min(
            self.max_players,
            len(self.players),
            self.MAX_COMPONENTS - self.CONTROL_COUNT,
        )
        for i in random.sample(range(count), k=min(count, self.max_players)):
            self.selected.add(i)
        for child in self.children:
            if isinstance(child, PlayerButton):
                child.set_selected(child.idx in self.selected)
        self.refresh_confirm()
        await interaction.response.edit_message(
            content=f"🎲 {len(self.selected)} Spieler zufällig ausgewählt.", view=self
        )


# ================= Turnier-Registrierung =================


def _signup_status_text(status: str) -> str:
    if status == "inserted":
        return "eingetragen"
    if status == "updated":
        return "aktualisiert"
    return "bereits unveraendert vorhanden"


class RestrictedUserView(discord.ui.View):
    def __init__(self, owner_id: int, *, timeout: float = TOURNAMENT_UI_TIMEOUT):
        super().__init__(timeout=timeout)
        self.owner_id = int(owner_id)

    async def interaction_check(self, interaction: discord.Interaction) -> bool:
        if interaction.user.id != self.owner_id:
            await interaction.response.send_message(
                "Dieses Menü gehört dir nicht. Bitte starte dein eigenes.",
                ephemeral=True,
            )
            return False
        return True


class SoloRankSelect(discord.ui.Select):
    def __init__(self):
        options = [
            discord.SelectOption(
                label=label,
                value=value,
                description=f"Setze {label} als Turnier-Rang",
            )
            for label, value, _ in tstore.rank_choices()
        ]
        super().__init__(
            placeholder="Wähle deinen aktuellen Rang...",
            min_values=1,
            max_values=1,
            options=options,
        )

    async def callback(self, interaction: discord.Interaction):
        view: SoloSignupView = self.view  # type: ignore
        view.selected_rank = self.values[0]
        await interaction.response.defer()


class SoloSignupView(RestrictedUserView):
    def __init__(self, owner_id: int, guild_id: int):
        super().__init__(owner_id)
        self.guild_id = int(guild_id)
        self.selected_rank: str | None = None
        self.add_item(SoloRankSelect())

    @discord.ui.button(label="Eintragen", style=discord.ButtonStyle.success, emoji="✅")
    async def submit_button(self, interaction: discord.Interaction, _: discord.ui.Button):
        if not self.selected_rank:
            await interaction.response.send_message(
                "Bitte wähle zuerst deinen Rang aus.",
                ephemeral=True,
            )
            return
        result = await tstore.upsert_signup_async(
            self.guild_id,
            interaction.user.id,
            registration_mode="solo",
            rank=self.selected_rank,
            team_id=None,
            assigned_by_admin=False,
        )
        text = (
            f"✅ Turnier-Eintrag {_signup_status_text(str(result.get('status')))}.\n"
            f"Modus: Solo\n"
            f"Rang: {tstore.rank_label(str(result.get('rank', self.selected_rank)))}"
        )
        await interaction.response.edit_message(content=text, embed=None, view=None)


class TeamNameModal(discord.ui.Modal, title="Team erstellen"):
    team_name = discord.ui.TextInput(
        label="Teamname",
        placeholder="z. B. Early Salty",
        min_length=tstore.TEAM_NAME_MIN,
        max_length=tstore.TEAM_NAME_MAX,
    )

    def __init__(self, parent_view: "TeamSignupView"):
        super().__init__(timeout=TOURNAMENT_UI_TIMEOUT)
        self.parent_view = parent_view

    async def on_submit(self, interaction: discord.Interaction):
        try:
            team = await tstore.get_or_create_team_async(
                self.parent_view.guild_id,
                str(self.team_name),
                created_by=interaction.user.id,
            )
        except ValueError as exc:
            await interaction.response.send_message(f"❌ {exc}", ephemeral=True)
            return
        self.parent_view.selected_team_id = int(team["id"])
        self.parent_view.selected_team_name = str(team["name"])
        state = "neu erstellt" if bool(team.get("created")) else "bereits vorhanden"
        await interaction.response.send_message(
            f"✅ Team **{team['name']}** ist {state} und ausgewählt.",
            ephemeral=True,
        )


class TeamSelect(discord.ui.Select):
    CREATE_VALUE = "__create__"

    def __init__(self, teams: list[dict[str, object]]):
        options: list[discord.SelectOption] = []
        for team in teams[:24]:
            team_id = int(team.get("id", 0) or 0)
            if team_id <= 0:
                continue
            team_name = str(team.get("name") or f"Team {team_id}")
            members = int(team.get("member_count", 0) or 0)
            options.append(
                discord.SelectOption(
                    label=team_name[:100],
                    value=str(team_id),
                    description=f"{members} Spieler bereits eingetragen",
                )
            )
        options.append(
            discord.SelectOption(
                label="Neues Team erstellen",
                value=self.CREATE_VALUE,
                description="Falls dein Team noch nicht existiert",
                emoji="➕",
            )
        )
        super().__init__(
            placeholder="Team wählen oder neu erstellen...",
            min_values=1,
            max_values=1,
            options=options,
        )

    async def callback(self, interaction: discord.Interaction):
        view: TeamSignupView = self.view  # type: ignore
        selected = self.values[0]
        if selected == self.CREATE_VALUE:
            await interaction.response.send_modal(TeamNameModal(view))
            return
        try:
            team_id = int(selected)
        except ValueError:
            await interaction.response.send_message("❌ Ungültige Team-Auswahl.", ephemeral=True)
            return
        view.selected_team_id = team_id
        view.selected_team_name = view.team_name_by_id.get(team_id)
        await interaction.response.defer()


class TeamRankSelect(discord.ui.Select):
    def __init__(self):
        options = [
            discord.SelectOption(
                label=label,
                value=value,
                description=f"Setze {label} als Turnier-Rang",
            )
            for label, value, _ in tstore.rank_choices()
        ]
        super().__init__(
            placeholder="Rang für Turnier-Eintrag auswählen...",
            min_values=1,
            max_values=1,
            options=options,
        )

    async def callback(self, interaction: discord.Interaction):
        view: TeamSignupView = self.view  # type: ignore
        view.selected_rank = self.values[0]
        await interaction.response.defer()


class TeamSignupView(RestrictedUserView):
    def __init__(self, owner_id: int, guild_id: int, teams: list[dict[str, object]]):
        super().__init__(owner_id)
        self.guild_id = int(guild_id)
        self.selected_rank: str | None = None
        self.selected_team_id: int | None = None
        self.selected_team_name: str | None = None
        self.team_name_by_id: dict[int, str] = {
            int(team["id"]): str(team.get("name") or "")
            for team in teams
            if team.get("id") is not None
        }
        self.add_item(TeamSelect(teams))
        self.add_item(TeamRankSelect())

    @discord.ui.button(label="Eintragen", style=discord.ButtonStyle.success, emoji="✅")
    async def submit_button(self, interaction: discord.Interaction, _: discord.ui.Button):
        if not self.selected_team_id:
            await interaction.response.send_message(
                "Bitte wähle zuerst ein Team aus oder erstelle eines.",
                ephemeral=True,
            )
            return
        if not self.selected_rank:
            await interaction.response.send_message(
                "Bitte wähle zuerst deinen Rang aus.",
                ephemeral=True,
            )
            return
        result = await tstore.upsert_signup_async(
            self.guild_id,
            interaction.user.id,
            registration_mode="team",
            rank=self.selected_rank,
            team_id=self.selected_team_id,
            assigned_by_admin=False,
        )
        team_name = str(
            result.get("team_name") or self.selected_team_name or f"Team {self.selected_team_id}"
        )
        text = (
            f"✅ Turnier-Eintrag {_signup_status_text(str(result.get('status')))}.\n"
            f"Modus: Team\n"
            f"Team: {team_name}\n"
            f"Rang: {tstore.rank_label(str(result.get('rank', self.selected_rank)))}"
        )
        await interaction.response.edit_message(content=text, embed=None, view=None)


class TournamentPanelView(discord.ui.View):
    def __init__(self):
        super().__init__(timeout=None)

    @discord.ui.button(
        label="Als Team eintragen",
        style=discord.ButtonStyle.primary,
        emoji="🛡️",
        custom_id="deadlock_tournament_entry_team",
    )
    async def team_entry_button(self, interaction: discord.Interaction, _: discord.ui.Button):
        cog = interaction.client.get_cog("DeadlockTeamBalancer")
        if not cog:
            await interaction.response.send_message(
                "❌ Turnier-Cog nicht verfügbar.", ephemeral=True
            )
            return
        await cog.open_tournament_team_entry(interaction)

    @discord.ui.button(
        label="Alleine eintragen",
        style=discord.ButtonStyle.secondary,
        emoji="🎯",
        custom_id="deadlock_tournament_entry_solo",
    )
    async def solo_entry_button(self, interaction: discord.Interaction, _: discord.ui.Button):
        cog = interaction.client.get_cog("DeadlockTeamBalancer")
        if not cog:
            await interaction.response.send_message(
                "❌ Turnier-Cog nicht verfügbar.", ephemeral=True
            )
            return
        await cog.open_tournament_solo_entry(interaction)


# ================= Haupt-Cog =================


@dataclass
class MatchInfo:
    guild_id: int
    team1_channel_id: int
    team2_channel_id: int
    players: list[int]
    started_at: datetime
    original_channel_id: int | None = None


class DeadlockTeamBalancer(commands.Cog):
    """Teambalancer: 2 Teams nach Rängen, strikt 2 VC in Kategorie MATCH_CATEGORY_ID, Auto-Move, volle Befehlsparität."""

    def __init__(self, bot: commands.Bot):
        self.bot = bot
        self._guild_locks: dict[int, asyncio.Lock] = {}
        self.active_matches: dict[str, MatchInfo] = {}
        self._match_counter = 0

    # ---------- Lifecycle ----------
    async def cog_load(self):
        await tstore.ensure_schema_async()
        # Persistente Panel-Buttons fuer Turnier-Anmeldungen
        if not getattr(self.bot, "_deadlock_tournament_panel_registered", False):
            self.bot.add_view(TournamentPanelView())
            self.bot._deadlock_tournament_panel_registered = True
        logger.info("DeadlockTeamBalancer bereit (DB verbunden, Turnier-Schema aktiv)")
        logger.info("DeadlockTeamBalancer Cog geladen")

    # ---------- Rank-Ermittlung ----------
    async def get_user_rank(self, member: discord.Member) -> tuple[str, int]:
        rn, rv = _rank_from_roles(member)
        if rv > 0:
            return rn, rv
        # Fallback to DB lookup using central DB
        return await _fetch_rank_from_db(member.id)

    async def open_tournament_solo_entry(self, interaction: discord.Interaction) -> None:
        if not interaction.guild:
            await interaction.response.send_message(
                "❌ Turnier-Anmeldung funktioniert nur auf dem Server.",
                ephemeral=True,
            )
            return
        await tstore.ensure_schema_async()
        view = SoloSignupView(interaction.user.id, interaction.guild.id)
        embed = discord.Embed(
            title="🎯 Solo-Turnieranmeldung",
            description="Wähle deinen aktuellen Deadlock-Rang und bestätige deinen Eintrag.",
            color=discord.Color.blurple(),
        )
        await interaction.response.send_message(embed=embed, view=view, ephemeral=True)

    async def open_tournament_team_entry(self, interaction: discord.Interaction) -> None:
        if not interaction.guild:
            await interaction.response.send_message(
                "❌ Turnier-Anmeldung funktioniert nur auf dem Server.",
                ephemeral=True,
            )
            return
        await tstore.ensure_schema_async()
        teams = await tstore.list_teams_async(interaction.guild.id)
        embed = discord.Embed(
            title="🛡️ Team-Turnieranmeldung",
            description=(
                "Wähle ein bestehendes Team oder erstelle ein neues Team.\n"
                "Danach Rang auswählen und Eintrag absenden."
            ),
            color=discord.Color.green(),
        )
        embed.add_field(
            name="Teams",
            value=f"Es gibt aktuell **{len(teams)}** Team(s) in der Datenbank.",
            inline=False,
        )
        if len(teams) > 24:
            embed.set_footer(
                text="Hinweis: Dropdown zeigt max. 24 Teams. Existiert dein Team nicht: Neues Team erstellen."
            )
        view = TeamSignupView(interaction.user.id, interaction.guild.id, teams)
        await interaction.response.send_message(embed=embed, view=view, ephemeral=True)

    # ---------- Commands ----------
    @commands.group(name="balance", aliases=["bal", "teams"], invoke_without_command=True)
    async def balance_root(self, ctx: commands.Context):
        emb = discord.Embed(
            title="⚖️ Deadlock Team Balancer",
            description="Teilt Spieler in **2 Teams** nach Rang auf und **erstellt immer 2 Voice-Channels** in der Match-Kategorie.",
            color=0x0099FF,
        )
        emb.add_field(
            name="Befehle",
            value=(
                "`!balance auto` – nur Anzeige (keine Channels)\n"
                "`!balance start` – Channels erstellen & Spieler moven\n"
                "`!balance manual @u1 …` – manuelle Auswahl\n"
                "`!balance voice` – Alias von auto\n"
                "`!balance status [@user]` – Rank-Status\n"
                "`!balance matches` – aktive Matches\n"
                "`!balance end <id> [skip_debrief]` – Match beenden (+ Nachbesprechung)\n"
                "`!balance cleanup <hours>` – alte Matches löschen\n"
                "`!balance turnierpanel` – Turnier-Menü posten (Admin)\n"
                "`!balance turnierstatus` – dein Turnier-Eintrag\n"
                "`!balance austragen` – Turnier-Eintrag entfernen"
            ),
            inline=False,
        )
        emb.add_field(
            name="Spielerzahl",
            value="Min. 4 Spieler, max. 12 (6v6). Bei >12: interaktive Auswahl.",
            inline=False,
        )
        await ctx.send(embed=emb)

    @balance_root.command(name="auto")
    async def balance_auto(self, ctx: commands.Context):
        members = self._voice_members(ctx)
        if len(members) < 4:
            await ctx.send(f"❌ Mindestens 4 Spieler benötigt (aktuell: {len(members)})")
            return
        players: list[tuple[discord.Member, str, int]] = []
        for m in members:
            nm, val = await self.get_user_rank(m)
            players.append((m, nm, val))
        players.sort(key=lambda x: x[2], reverse=True)
        team_a, team_b = _best_split([(m, v) for (m, _n, v) in players])
        embed = _team_embed(team_a, team_b, "🎯 Vorschau – Team Balance (ohne Move)")
        await ctx.send(embed=embed)

    @balance_root.command(name="voice")
    async def balance_voice(self, ctx: commands.Context):
        await self.balance_auto(ctx)

    @balance_root.command(name="manual")
    async def balance_manual(self, ctx: commands.Context, *members: discord.Member):
        if len(members) < 4:
            await ctx.send(f"❌ Mindestens 4 Spieler benötigt (angegeben: {len(members)})")
            return
        if len(members) > TEAM_SIZE_CAP * 2:
            await ctx.send(
                f"❌ Maximal {TEAM_SIZE_CAP * 2} Spieler unterstützt (angegeben: {len(members)})"
            )
            return
        players: list[tuple[discord.Member, str, int]] = []
        for m in members:
            nm, val = await self.get_user_rank(m)
            players.append((m, nm, val))
        await self._run_balance_and_start(ctx, players)

    @balance_root.command(name="start")
    async def balance_start(self, ctx: commands.Context):
        """Erstellt ZWINGEND zwei Voice-Channels in der Match-Kategorie und moved die Spieler in 2 Teams."""
        members = self._voice_members(ctx)
        if len(members) < 4:
            await ctx.send(f"❌ Mindestens 4 Spieler benötigt (aktuell: {len(members)})")
            return

        players: list[tuple[discord.Member, str, int]] = []
        for m in members:
            nm, val = await self.get_user_rank(m)
            players.append((m, nm, val))

        if len(players) > TEAM_SIZE_CAP * 2:
            embed = discord.Embed(
                title="👥 Zu viele Spieler",
                description=f"Es sind **{len(players)}** Spieler im Channel – wähle bis zu **{TEAM_SIZE_CAP * 2}** Spieler aus.",
                color=discord.Color.orange(),
            )
            lines = []
            for i, (mem, nm, val) in enumerate(players[:20], 1):
                lines.append(f"{i}. **{mem.display_name}** — {nm} ({val})")
            if len(players) > 20:
                lines.append(f"... und {len(players) - 20} weitere")
            embed.add_field(name="Spieler (Ausschnitt)", value="\n".join(lines), inline=False)
            view = SelectionView(ctx, players, max_players=TEAM_SIZE_CAP * 2)
            await ctx.send(embed=embed, view=view)
            return

        await self._run_balance_and_start(ctx, players)

    @balance_root.command(name="turnierpanel", aliases=["tournamentpanel", "cupmenu"])
    @commands.has_permissions(manage_guild=True)
    async def balance_tournament_panel(self, ctx: commands.Context):
        if not ctx.guild:
            await ctx.send("❌ Dieser Befehl funktioniert nur in einem Server.")
            return
        await tstore.ensure_schema_async()
        embed = discord.Embed(
            title="🏆 Deadlock Turnier-Anmeldung",
            description=(
                "Trage dich hier für das Turnier ein:\n"
                "• Als Team (Team wählen oder neu erstellen)\n"
                "• Oder alleine\n\n"
                "Der aktuelle Rang wird über das Dropdown abgefragt."
            ),
            color=discord.Color.gold(),
        )
        embed.set_footer(
            text="Doppelte Einträge und doppelte Team-Namen werden automatisch verhindert."
        )
        await ctx.send(embed=embed, view=TournamentPanelView())

    @balance_root.command(name="turnierstatus", aliases=["cupstatus"])
    async def balance_tournament_status(
        self, ctx: commands.Context, member: discord.Member | None = None
    ):
        if not ctx.guild:
            await ctx.send("❌ Dieser Befehl funktioniert nur in einem Server.")
            return
        target = member or ctx.author
        if target.id != ctx.author.id:
            perms = ctx.author.guild_permissions
            if not (perms.manage_guild or perms.administrator):
                await ctx.send("❌ Du darfst nur deinen eigenen Turnierstatus anzeigen.")
                return

        await tstore.ensure_schema_async()
        signup = await tstore.get_signup_async(ctx.guild.id, target.id)
        if not signup:
            await ctx.send(f"ℹ️ {target.mention} hat aktuell keinen Turnier-Eintrag.")
            return

        mode = str(signup.get("registration_mode", "solo"))
        rank_key = str(signup.get("rank", "initiate"))
        rank_num = int(signup.get("rank_value", tstore.rank_value(rank_key)) or 0)
        team_name = signup.get("team_name")
        assigned = bool(int(signup.get("assigned_by_admin", 0) or 0))

        emb = discord.Embed(
            title=f"🏆 Turnierstatus: {target.display_name}",
            color=discord.Color.orange(),
        )
        emb.add_field(name="Modus", value="Team" if mode == "team" else "Solo", inline=True)
        emb.add_field(
            name="Rang",
            value=f"{tstore.rank_label(rank_key)} ({rank_num})",
            inline=True,
        )
        emb.add_field(
            name="Team",
            value=str(team_name) if team_name else "Kein Team zugewiesen",
            inline=True,
        )
        if assigned:
            emb.set_footer(text="Hinweis: Team-Zuweisung wurde im Admin-Panel gesetzt.")
        await ctx.send(embed=emb)

    @balance_root.command(name="turnierliste", aliases=["cuplist"])
    @commands.has_permissions(manage_guild=True)
    async def balance_tournament_list(self, ctx: commands.Context):
        if not ctx.guild:
            await ctx.send("❌ Dieser Befehl funktioniert nur in einem Server.")
            return
        await tstore.ensure_schema_async()
        signups = await tstore.list_signups_async(ctx.guild.id)
        summary = await tstore.summary_async(ctx.guild.id)
        if not signups:
            await ctx.send("📭 Keine Turnier-Anmeldungen vorhanden.")
            return

        emb = discord.Embed(
            title="📋 Turnier-Anmeldungen",
            description=(
                f"Gesamt: **{summary['signups_total']}** | "
                f"Solo: **{summary['solo_count']}** | "
                f"Team: **{summary['team_count']}** | "
                f"Teams: **{summary['teams_count']}**"
            ),
            color=discord.Color.teal(),
        )
        lines: list[str] = []
        for row in signups[:20]:
            user_id = int(row.get("user_id") or 0)
            member = ctx.guild.get_member(user_id)
            display = member.display_name if member else str(user_id)
            mode = "Team" if str(row.get("registration_mode")) == "team" else "Solo"
            rank_lbl = tstore.rank_label(str(row.get("rank", "initiate")))
            team = str(row.get("team_name") or "—")
            lines.append(f"• **{display}** | {mode} | {rank_lbl} | {team}")
        if len(signups) > 20:
            lines.append(f"... und {len(signups) - 20} weitere")
        emb.add_field(name="Einträge", value="\n".join(lines), inline=False)
        await ctx.send(embed=emb)

    @balance_root.command(name="austragen", aliases=["turnierexit", "unregister"])
    async def balance_tournament_unregister(self, ctx: commands.Context):
        if not ctx.guild:
            await ctx.send("❌ Dieser Befehl funktioniert nur in einem Server.")
            return
        await tstore.ensure_schema_async()
        removed = await tstore.remove_signup_async(ctx.guild.id, ctx.author.id)
        if removed:
            await ctx.send("✅ Dein Turnier-Eintrag wurde entfernt.")
        else:
            await ctx.send("ℹ️ Du warst nicht als Turnier-Teilnehmer eingetragen.")

    @balance_root.command(name="status")
    async def balance_status(self, ctx: commands.Context, member: discord.Member | None = None):
        member = member or ctx.author
        nm, val = await self.get_user_rank(member)
        role_nm, role_val = _rank_from_roles(member)
        db_nm, db_val = await _fetch_rank_from_db(member.id)
        emb = discord.Embed(
            title=f"📊 Rank-Status: {member.display_name}", color=discord.Color.blue()
        )
        emb.add_field(name="🎯 Aktueller Rank", value=f"**{nm}** ({val})", inline=True)
        emb.add_field(
            name="🎭 Discord-Rollen",
            value=f"{role_nm} ({role_val})" if role_val else "Keine Rank-Rolle",
            inline=True,
        )
        emb.add_field(
            name="🗃️ DB-Fallback",
            value=f"{db_nm} ({db_val})" if db_val else "Nicht in DB",
            inline=True,
        )
        await ctx.send(embed=emb)

    @balance_root.command(name="matches")
    async def balance_matches(self, ctx: commands.Context):
        if not self.active_matches:
            await ctx.send("📭 Keine aktiven Matches")
            return
        emb = discord.Embed(title="🎮 Aktive Deadlock Matches", color=discord.Color.green())
        for match_id, info in self.active_matches.items():
            guild = self.bot.get_guild(info.guild_id)
            if not guild:
                continue
            lines = []
            for ch_id in (info.team1_channel_id, info.team2_channel_id):
                ch = guild.get_channel(ch_id)
                if ch:
                    count = len([m for m in ch.members if not m.bot])
                    lines.append(f"{ch.name}: {count} Spieler")
                else:
                    lines.append(f"{ch_id}: gelöscht")
            dur = datetime.utcnow() - info.started_at
            emb.add_field(
                name=f"Match {match_id}",
                value=f"Dauer: {dur.seconds // 60}min {dur.seconds % 60}s\n" + "\n".join(lines),
                inline=False,
            )
        await ctx.send(embed=emb)

    @balance_root.command(name="cleanup")
    @commands.has_permissions(manage_channels=True)
    async def balance_cleanup(self, ctx: commands.Context, hours: int = 2):
        if hours < 1 or hours > 24:
            await ctx.send("❌ Stunden müssen zwischen 1–24 liegen")
            return
        cutoff = datetime.utcnow() - timedelta(hours=hours)
        targets = [(mid, mi) for mid, mi in self.active_matches.items() if mi.started_at < cutoff]
        if not targets:
            await ctx.send(f"🧹 Keine Matches älter als {hours}h gefunden")
            return
        deleted_ch = 0
        for match_id, info in targets:
            for ch_id in (info.team1_channel_id, info.team2_channel_id):
                ch = ctx.guild.get_channel(ch_id)
                if isinstance(ch, (discord.VoiceChannel, discord.StageChannel)):
                    try:
                        await ch.delete(reason=f"Auto-Cleanup (> {hours}h)")
                        deleted_ch += 1
                        await asyncio.sleep(0.4)
                    except discord.Forbidden:
                        logger.debug("Cleanup: delete forbidden for channel %s", ch_id)
                    except discord.HTTPException as e:
                        logger.debug("Cleanup: HTTP error deleting %s: %r", ch_id, e)
                    except Exception as e:
                        logger.debug("Cleanup: unexpected error deleting %s: %r", ch_id, e)
            self.active_matches.pop(match_id, None)
        await ctx.send(f"🧹 {len(targets)} Matches bereinigt ({deleted_ch} Channels gelöscht)")

    @balance_root.command(name="end")
    async def balance_end(
        self,
        ctx: commands.Context,
        match_id: str | None = None,
        skip_debrief: bool | None = False,
    ):
        if not match_id:
            await ctx.send("❌ Bitte Match-ID angeben: `!balance end <id>`")
            return
        info = self.active_matches.get(match_id)
        if not info:
            await ctx.send(f"❌ Match `{match_id}` nicht gefunden.")
            return

        # 1) Optionale Nachbesprechungs-Lane in gleicher Kategorie
        debrief_ch = None
        moved = 0
        move_fail: list[str] = []
        if not skip_debrief:
            cat = ctx.guild.get_channel(MATCH_CATEGORY_ID)
            if isinstance(cat, discord.CategoryChannel):
                try:
                    debrief_ch = await ctx.guild.create_voice_channel(
                        name=f"💬 Nachbesprechung • {match_id}",
                        category=cat,
                        reason=f"Match {match_id} Nachbesprechung",
                    )
                except Exception as e:
                    logger.warning(f"Debrief-VC create failed: {e}")

            if debrief_ch:
                for uid in info.players:
                    try:
                        m = ctx.guild.get_member(uid)
                        if (
                            m
                            and m.voice
                            and m.voice.channel
                            and m.voice.channel.id in (info.team1_channel_id, info.team2_channel_id)
                        ):
                            await m.move_to(
                                debrief_ch, reason=f"Match {match_id} beendet – Debrief"
                            )
                            moved += 1
                            await asyncio.sleep(0.3)
                    except discord.Forbidden:
                        move_fail.append(f"<@{uid}> (keine Berechtigung)")
                    except discord.HTTPException as e:
                        move_fail.append(f"<@{uid}> (HTTP: {e})")
                    except Exception as e:
                        move_fail.append(f"<@{uid}> (Fehler: {e})")

        # 2) Team-Channels löschen
        deleted = []
        failed = []
        for ch_id in (info.team1_channel_id, info.team2_channel_id):
            ch = ctx.guild.get_channel(ch_id)
            if isinstance(ch, (discord.VoiceChannel, discord.StageChannel)):
                try:
                    await ch.delete(reason=f"Match {match_id} beendet")
                    deleted.append(ch.name)
                    await asyncio.sleep(0.4)
                except Exception as e:
                    failed.append(f"{ch.mention if hasattr(ch, 'mention') else ch.id} ({e})")
            else:
                failed.append(f"{ch_id} (nicht gefunden)")

        self.active_matches.pop(match_id, None)

        # 3) Ergebnis
        emb = discord.Embed(
            title=f"🏁 Match {match_id} beendet",
            color=discord.Color.green() if debrief_ch else discord.Color.orange(),
        )
        if debrief_ch:
            emb.add_field(
                name="💬 Nachbesprechungs-Lane",
                value=f"{debrief_ch.mention}\nSpieler bewegt: {moved}/{len(info.players)}",
                inline=False,
            )
            if move_fail:
                emb.add_field(
                    name="⚠️ Nicht bewegt",
                    value="\n".join(move_fail[:6])
                    + (f"\n… und {len(move_fail) - 6} weitere" if len(move_fail) > 6 else ""),
                    inline=False,
                )
        if deleted:
            emb.add_field(name="🗑️ Gelöschte Team-Channels", value="\n".join(deleted), inline=False)
        if failed:
            emb.add_field(name="⚠️ Nicht gelöscht", value="\n".join(failed), inline=False)
        dur = datetime.utcnow() - info.started_at
        emb.add_field(
            name="📊 Dauer",
            value=f"{dur.seconds // 60}min {dur.seconds % 60}s",
            inline=True,
        )
        emb.add_field(name="👥 Spieler", value=str(len(info.players)), inline=True)
        await ctx.send(embed=emb)

    # ---------- Kernablauf ----------
    async def _run_balance_and_start(
        self, ctx: commands.Context, players_in: list[tuple[discord.Member, str, int]]
    ):
        players_mv: list[tuple[discord.Member, int]] = sorted(
            [(m, v) for (m, _nm, v) in players_in], key=lambda t: t[1], reverse=True
        )

        team_a, team_b = _best_split(players_mv)

        # Immer zwei Channels in der Kategorie erstellen (Pflicht)
        cat = ctx.guild.get_channel(MATCH_CATEGORY_ID)
        if not isinstance(cat, discord.CategoryChannel):
            await ctx.send(
                f"❌ Kategorie `{MATCH_CATEGORY_ID}` nicht gefunden oder keine Kategorie."
            )
            return

        match_id = self._next_match_id()
        team1_name = f"🟠 Team Amber • {match_id}"
        team2_name = f"🔵 Team Sapphire • {match_id}"
        lock = self._guild_locks.setdefault(ctx.guild.id, asyncio.Lock())

        async with lock:
            try:
                ch1 = await ctx.guild.create_voice_channel(
                    name=team1_name, category=cat, reason=f"Match {match_id}"
                )
                await asyncio.sleep(0.4)
                ch2 = await ctx.guild.create_voice_channel(
                    name=team2_name, category=cat, reason=f"Match {match_id}"
                )
            except discord.Forbidden:
                await ctx.send("❌ Keine Berechtigung, Voice-Channels zu erstellen.")
                return
            except Exception as e:
                await ctx.send(f"❌ Konnte Channels nicht erstellen: {e}")
                return

        # Spieler bewegen
        moved_a, moved_b, fail = await self._move_teams(team_a, team_b, ch1, ch2)

        # persistieren
        orig = (
            ctx.author.voice.channel.id if ctx.author.voice and ctx.author.voice.channel else None
        )
        self.active_matches[match_id] = MatchInfo(
            guild_id=ctx.guild.id,
            team1_channel_id=ch1.id,
            team2_channel_id=ch2.id,
            players=[m.id for m, _ in (team_a + team_b)],
            started_at=datetime.utcnow(),
            original_channel_id=orig,
        )

        # Embed mit Balance & Move-Ergebnis
        embed = _team_embed(
            team_a, team_b, f"🎮 Match {match_id} – Teams erstellt & Spieler verschoben"
        )
        embed.add_field(
            name="Move",
            value=(
                f"{ch1.mention}: {moved_a}/{len(team_a)}\n{ch2.mention}: {moved_b}/{len(team_b)}"
            ),
            inline=False,
        )
        if fail:
            embed.add_field(
                name="⚠️ Nicht bewegt",
                value="\n".join(fail[:6])
                + (f"\n… und {len(fail) - 6} weitere" if len(fail) > 6 else ""),
                inline=False,
            )
        embed.set_footer(text=f"Match-ID: {match_id} • Beenden: !balance end {match_id}")
        await ctx.send(embed=embed)

    async def _move_teams(
        self,
        team_a: list[tuple[discord.Member, int]],
        team_b: list[tuple[discord.Member, int]],
        ch1: discord.VoiceChannel,
        ch2: discord.VoiceChannel,
    ) -> tuple[int, int, list[str]]:
        moved_a = moved_b = 0
        failed: list[str] = []

        order = [(team_a[i], ch1) for i in range(len(team_a))] + [
            (team_b[i], ch2) for i in range(len(team_b))
        ]
        for (m, _), target in order:
            try:
                if m.voice and m.voice.channel:
                    await m.move_to(target, reason="Deadlock Team Balance")
                    if target.id == ch1.id:
                        moved_a += 1
                    else:
                        moved_b += 1
                else:
                    failed.append(f"{m.display_name} (nicht in Voice)")
            except discord.Forbidden:
                failed.append(f"{m.display_name} (keine Berechtigung)")
            except discord.HTTPException as e:
                failed.append(f"{m.display_name} (HTTP: {e})")
            except Exception as e:
                failed.append(f"{m.display_name} (Fehler: {e})")
            await asyncio.sleep(MOVE_SLEEP)

        return moved_a, moved_b, failed

    # ---------- Utils ----------
    def _voice_members(self, ctx: commands.Context) -> list[discord.Member]:
        if not ctx.author.voice or not isinstance(ctx.author.voice.channel, discord.VoiceChannel):
            return []
        return [m for m in ctx.author.voice.channel.members if not m.bot]

    def _next_match_id(self) -> str:
        self._match_counter += 1
        return f"{self._match_counter:03d}"


# ================= Setup =================


async def setup(bot: commands.Bot):
    await bot.add_cog(DeadlockTeamBalancer(bot))
    logger.info("DeadlockTeamBalancer (paritäts-komplett) geladen")
