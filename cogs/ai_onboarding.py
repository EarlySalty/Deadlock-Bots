"""AI-gestütztes Onboarding mit kurzen Fragen und personalisierter Tour."""

from __future__ import annotations

import json
import logging
import os
from dataclasses import dataclass
from textwrap import dedent
from typing import Any

import discord
from discord.ext import commands

from cogs import privacy_core as privacy
from service import db as service_db

log = logging.getLogger(__name__)

PRIMARY_MODEL = os.getenv("DEADLOCK_ONBOARD_MODEL", "gpt-5.2")
MAX_OUTPUT_TOKENS = int(os.getenv("DEADLOCK_ONBOARD_TOKENS", "700") or "700")
GUILD_ID = 1289721245281292288

# Klickbare Channel-Links (so weit bekannt)
LFG_CHANNEL_URL = f"https://discord.com/channels/{GUILD_ID}/1376335502919335936"
TEMPVOICE_PANEL_URL = f"https://discord.com/channels/{GUILD_ID}/1371927143537315890"
FEEDBACK_CHANNEL_URL = f"https://discord.com/channels/{GUILD_ID}/1289721245281292291"
RULES_CHANNEL_URL = f"https://discord.com/channels/{GUILD_ID}/1315684135175716975"

# Rollen-IDs (Discord Onboarding / Pings / Präferenzen)
ROLE_IGNORE_ID = 1304216250649415771  # Unwichtig: ignorieren
ROLE_STREAMER_ONBOARD_ID = 1468365558293598268
ROLE_STREAMER_PARTNER_ID = 1411798947936342097
ROLE_LFG_PING_ID = 1407086020331311144
ROLE_CUSTOM_GAMES_PING_ID = 1407085699374649364
ROLE_PATCHNOTES_PING_ID = 1330994309524357140
ROLE_RANKED_ID = 1420466763262591120
ROLE_CASUAL_ID = 1420466468746690621

ROLE_LABELS: dict[int, str] = {
    ROLE_STREAMER_ONBOARD_ID: "Streamer Onboarding Rolle",
    ROLE_STREAMER_PARTNER_ID: "Streamer Partner Rolle",
    ROLE_LFG_PING_ID: "Spieler-Suche Ping Rolle",
    ROLE_CUSTOM_GAMES_PING_ID: "Custom Games Ping Rolle",
    ROLE_PATCHNOTES_PING_ID: "Patchnotes Ping Rolle",
    ROLE_RANKED_ID: "Ranked/Rang-Spieler Rolle",
    ROLE_CASUAL_ID: "Casual/Spaß-Spieler Rolle",
}

STREAMING_KEYWORDS = (
    "stream",
    "streamer",
    "twitch",
    "kick",
    "youtube",
    "yt",
    "livestream",
    "live gehen",
    "obs",
)

SYSTEM_PROMPT = dedent(
    """
    Du bist der herzliche Onboarding-Guide der Deutschen Deadlock Community.
    Antworte immer auf Deutsch.

    Ziele:
    - Begrüße den User warm und freundlich (max. 2 Sätze).
    - Spiegle grob den Stil des Users (locker/kurz/ggf. mit wenigen Emojis), bleibe aber immer positiv und einladend.
    - Gib eine kurze, personalisierte Tour, nur das Relevante aus dem Kontext auswählen.
    - Schlage 2–3 konkrete nächste Schritte vor (Kanäle/Befehle), passend zu den Antworten.
    - Formatiere klar: kurze Absätze, keine Kanal-Listen als Fließtext.
    - Nutze höchstens 4 relevante Kanäle insgesamt.
    - Pro Bullet/Schritt maximal 1 Kanal.
    - Sei kompakt: 6–9 Sätze gesamt, kein Roman.
    - Nutze nur den gegebenen Kontext, wenn du etwas nicht weißt, beantworte es nicht.

    """
).strip()

SERVER_CONTEXT = dedent(
    """
    Server: Deutsche Deadlock Community (Discord)
    Wichtige Bereiche:
    - #📝patchnotes - Patchnotes auf Deutsch 
    - #📢ankündigungen: Updates & News.
    - #💬build-discussion - Für fragen zu Builds wie man z.b. was baut auf Heros oder sowas.
    - #🎮spieler-suche (LFG): Leute für Runden finden.
    - #🚧sprach-kanal-verwalten: eigene Lanes erstellen & verwalten (lanes=sprachkanal).
    - #🏆rang-auswahl: Rang-Rolle wählen (hilft beim Matchmaking).
    - #🛠️ich-brauch-einen-coach: Hilfe/Coaching anfragen.
    - #📺clip-submission: Highlights teilen.
    - #❓feedback-kanal: offen Feedback geben.
    - #🎥twitch Ankündigungen wer gerade Live ist von unsern Streamern.
    - #🎟️ticket-eröffnen: Support Ticket aufmachen und mit einem Moderator über dein Anliegen sprechen 
    - #🗝️beta-zugang wenn die Person noch keinen zugang zu Deadlock hat aber ihn braucht1407085699374649364
    - #🧩custom-games-chat wenn wir Custom Games machen oder du welche vorschlagen willst :)

    Rollen & Pings (optional):
    - Patchnotes Ping Rolle: bekommt Benachrichtigungen zu #📝patchnotes
    - Spieler-Suche Ping Rolle: passend zu #🎮spieler-suche (LFG)
    - Custom Games Ping Rolle: passend zu #🧩custom-games-chat
    - Ranked/Rang-Spieler Rolle: hilft bei Ranked/Competitiv Lanes
    - Casual/Spaß-Spieler Rolle: passt zur Spaß Lane
    - Streamer Onboarding / Streamer Partner: für Streamer-Setup via /streamer
    
    Sprachkanäle:
    - #📍Sammelpunkt - für die Custom Games zum Sammeln halt sammelpunkt
    - #🏆Coaching Lane🏆 - Sprachkanal für zum Coachen
    - #🆕 Neue Spieler Lane - Falls du noch neu im Game bist, wenig erfahrung hast oder wenig spielst hast du hier eine Speziell lane nur für Spieler in eurem Rank.
    - #➕Street Brawl Lanes - für den Modus Street brawl.
    - #➕Spaß Lane öffnen - Für entspannte runden ohne Rang begrenzung und ohne Rang druck. WICHTIG: Hier steht auch ein Rang dabei, der Dient aber nur als Richtungsgeber in welche, Rang bereich wir uns bewegen. Joinen kannst du Trotzdem.
    - #🗨️Off Topic Voice - Erklärt der Name von selbst, zum Quatschen und so für Themen die vielleicht nichts mit Deadlock zu tun haben.
    - #➕ Ranked/Competitiv Lane öffnen - Eingeschränkt auf deinen Rang bereich das man einigermaßen gleich gute Teammates hat udn der Skill diff nicht zu groß ist.

    Nützliche Bots/Commands:
    - /streamer für das Streamer-Partner-Setup wenn jemand Streamer ist kann er Streamer partner werden.

    Regeln (Kurz):
    - Respektvoll, keine Beleidigungen/Hassrede.
    - Kein Spam/keine Fremdwerbung
    - Kein NSFW.
    """
).strip()

NS_PERSIST_VIEWS = "ai_onboarding:persistent_views"
NS_SESSION_LOG = "ai_onboarding:sessions"


@dataclass
class UserAnswers:
    interests: str
    expectations: str
    style: str

    def as_prompt_block(self) -> str:
        return dedent(
            f"""
            Nutzer-Antworten:
            - Interessen: {self.interests or "-"}
            - Erwartungen: {self.expectations or "-"}
            - Stil-Hinweis/Art zu schreiben: {self.style or "-"}
            """
        ).strip()


def _looks_like_streamer(answers: UserAnswers) -> bool:
    text = " ".join(
        part.strip()
        for part in (answers.interests, answers.expectations, answers.style)
        if part and part.strip()
    ).lower()
    if not text:
        return False
    return any(keyword in text for keyword in STREAMING_KEYWORDS)


def _build_role_context_block(user: discord.abc.User, answers: UserAnswers) -> str:
    streaming_hint = _looks_like_streamer(answers)
    member = user if isinstance(user, discord.Member) else None

    if not member:
        role_lines = ["- (Rollen nicht verfügbar)"]
        hint_lines = []
        if streaming_hint:
            hint_lines.append(
                "- Streaming erkannt: Streamer-Partner explizit vorschlagen und /streamer nennen."
            )
        header = "Rollen-Kontext (Discord Onboarding):"
        context = "\n".join(role_lines)
        if hint_lines:
            hints = "\n".join(hint_lines)
            return f"{header}\n{context}\n\nHinweise:\n{hints}"
        return f"{header}\n{context}"

    role_ids = {role.id for role in member.roles}
    role_ids.discard(ROLE_IGNORE_ID)

    streamer_onboarding = ROLE_STREAMER_ONBOARD_ID in role_ids
    streamer_partner = ROLE_STREAMER_PARTNER_ID in role_ids
    lfg_ping = ROLE_LFG_PING_ID in role_ids
    custom_games_ping = ROLE_CUSTOM_GAMES_PING_ID in role_ids
    patchnotes_ping = ROLE_PATCHNOTES_PING_ID in role_ids
    ranked_role = ROLE_RANKED_ID in role_ids
    casual_role = ROLE_CASUAL_ID in role_ids

    role_lines = []
    for role_id, label in ROLE_LABELS.items():
        if role_id in role_ids:
            role_lines.append(f"- {label}: ja")
    if not role_lines:
        role_lines.append("- (keine relevanten Rollen erkannt)")

    hint_lines = []
    if (streaming_hint or streamer_onboarding) and not streamer_partner:
        hint_lines.append(
            "- Streaming erkannt: Streamer-Partner explizit vorschlagen und /streamer nennen."
        )
    if lfg_ping:
        hint_lines.append("- Spieler-Suche Ping: #🎮spieler-suche erwähnen.")
    if custom_games_ping:
        hint_lines.append("- Custom Games Ping: #🧩custom-games-chat und #📍Sammelpunkt erwähnen.")
    if patchnotes_ping:
        hint_lines.append("- Patchnotes Ping: #📝patchnotes erwähnen.")
    if ranked_role:
        hint_lines.append("- Ranked/Rang: #🏆rang-auswahl und Ranked/Competitiv Lane erwähnen.")
    if casual_role:
        hint_lines.append("- Casual/Spaß: Spaß Lane erwähnen.")

    header = "Rollen-Kontext (Discord Onboarding):"
    context = "\n".join(role_lines)
    if hint_lines:
        hints = "\n".join(hint_lines)
        return f"{header}\n{context}\n\nHinweise:\n{hints}"
    return f"{header}\n{context}"


class QuickActionsView(discord.ui.View):
    def __init__(self, *, allowed_user_id: int | None):
        super().__init__(timeout=1200)
        self.allowed_user_id = allowed_user_id

        # Link-Buttons (kein Custom-ID nötig)
        self.add_item(
            discord.ui.Button(
                label="Spieler-Suche",
                url=LFG_CHANNEL_URL,
                style=discord.ButtonStyle.link,
                emoji="🎮",
            )
        )
        self.add_item(
            discord.ui.Button(
                label="Sprachkanal-Panel",
                url=TEMPVOICE_PANEL_URL,
                style=discord.ButtonStyle.link,
                emoji="🛠️",
            )
        )
        self.add_item(
            discord.ui.Button(
                label="Feedback Hub",
                url=FEEDBACK_CHANNEL_URL,
                style=discord.ButtonStyle.link,
                emoji="💬",
            )
        )
        self.add_item(
            discord.ui.Button(
                label="Regelwerk",
                url=RULES_CHANNEL_URL,
                style=discord.ButtonStyle.link,
                emoji="📜",
            )
        )

    @discord.ui.button(
        label="Regeln gelesen ✅",
        style=discord.ButtonStyle.success,
        custom_id="aiob:rules_confirm",
        row=2,
    )
    async def confirm_rules(self, interaction: discord.Interaction, _button: discord.ui.Button):
        if self.allowed_user_id and interaction.user.id != self.allowed_user_id:
            await interaction.response.send_message(
                "Dieses Onboarding gehört jemand anderem – bitte den eigenen Button nutzen.",
                ephemeral=True,
            )
            return

        guild = interaction.guild or getattr(interaction.channel, "guild", None)
        member = interaction.user if isinstance(interaction.user, discord.Member) else None
        if not guild or not member:
            await interaction.response.send_message(
                "Ich konnte dich gerade nicht als Server-Mitglied zuordnen. Probier es kurz später erneut.",
                ephemeral=True,
            )
            return

        try:
            # Lazy import um zyklische Abhängigkeiten zu vermeiden
            from cogs.welcome_dm.base import ONBOARD_COMPLETE_ROLE_ID
        except Exception:
            ONBOARD_COMPLETE_ROLE_ID = None  # type: ignore[assignment]

        if ONBOARD_COMPLETE_ROLE_ID:
            role = guild.get_role(ONBOARD_COMPLETE_ROLE_ID)
        else:
            role = None

        if role:
            try:
                await member.add_roles(role, reason="AI Onboarding: Regeln bestätigt")
            except Exception as exc:  # pragma: no cover - defensive logging
                log.warning("Konnte ONBOARD-Rolle nicht setzen (%s): %s", member.id, exc)
                await interaction.response.send_message(
                    "Ich konnte die Onboarding-Rolle nicht setzen. Bitte kurz dem Team Bescheid geben.",
                    ephemeral=True,
                )
                return

        if not interaction.response.is_done():
            await interaction.response.send_message(
                "Danke! Viel Spaß auf dem Server. 😊", ephemeral=True
            )
        else:
            await interaction.followup.send("Danke! Viel Spaß auf dem Server. 😊", ephemeral=True)


class OnboardingQuestionsModal(discord.ui.Modal):
    """Fragt die 2-3 Kerninfos ab, damit die KI personalisieren kann."""

    def __init__(
        self,
        cog: AIOnboarding,
        *,
        allowed_user_id: int | None,
        thread_id: int | None,
    ):
        super().__init__(title="Dein Start auf dem Server", timeout=None)
        self.cog = cog
        self.allowed_user_id = allowed_user_id
        self.thread_id = thread_id

        self.interests = discord.ui.TextInput(
            label="Worauf hast du hier Lust?",
            placeholder="z. B. entspannte Runden, Ranked, Streams, neue Leute …",
            required=True,
            max_length=200,
        )
        self.expectations = discord.ui.TextInput(
            label="Was erhoffst du dir vom Server?",
            placeholder="Was soll dir der Server bringen?",
            required=True,
            max_length=300,
            style=discord.TextStyle.long,
        )
        self.style = discord.ui.TextInput(
            label="Wie schreibst du am liebsten?",
            placeholder="Locker/kurz/mit Emojis? Sag gern wie du tickst.",
            required=False,
            max_length=200,
        )

        self.add_item(self.interests)
        self.add_item(self.expectations)
        self.add_item(self.style)

    async def on_submit(
        self, interaction: discord.Interaction
    ) -> None:  # pragma: no cover - Discord runtime
        if self.allowed_user_id and interaction.user.id != self.allowed_user_id:
            await interaction.response.send_message(
                "Dieses Onboarding gehört jemand anderem – bitte den eigenen Button nutzen.",
                ephemeral=True,
            )
            return

        await interaction.response.defer(thinking=True)
        answers = UserAnswers(
            interests=str(self.interests.value).strip(),
            expectations=str(self.expectations.value).strip(),
            style=str(self.style.value).strip(),
        )

        text, meta = await self.cog.generate_personalized_text(
            answers=answers,
            user=interaction.user,
        )

        if not privacy.is_opted_out(interaction.user.id):
            await self.cog._log_session(
                user_id=interaction.user.id,
                thread_id=self.thread_id,
                answers=answers,
                llm_meta=meta,
            )

        embed = discord.Embed(
            title="Dein persönlicher Einstieg",
            description=text,
            colour=discord.Colour.blurple(),
        )
        embed.set_footer(text="Deadlock Master Bot · Onboarding")

        view = QuickActionsView(allowed_user_id=self.allowed_user_id)
        await interaction.followup.send(embed=embed, view=view)


class StartOnboardingView(discord.ui.View):
    """Start-Button für das Onboarding (persistent)."""

    def __init__(
        self,
        cog: AIOnboarding,
        *,
        allowed_user_id: int | None,
        thread_id: int | None,
        message_id: int | None = None,
    ):
        super().__init__(timeout=None)
        self.cog = cog
        self.allowed_user_id = allowed_user_id
        self.thread_id = thread_id
        self.message_id = message_id

    @discord.ui.button(
        label="Los geht's 🚀",
        style=discord.ButtonStyle.primary,
        custom_id="aiob:start",
    )
    async def start(self, interaction: discord.Interaction, _button: discord.ui.Button):
        if self.allowed_user_id and interaction.user.id != self.allowed_user_id:
            await interaction.response.send_message(
                "Dieses Onboarding gehört jemand anderem – bitte den eigenen Button nutzen.",
                ephemeral=True,
            )
            return

        if self.message_id:
            await self.cog._clear_persisted_view(self.message_id)

        modal = OnboardingQuestionsModal(
            self.cog,
            allowed_user_id=self.allowed_user_id,
            thread_id=self.thread_id,
        )
        await interaction.response.send_modal(modal)


class AIOnboarding(commands.Cog):
    """Fragt 2-3 Dinge ab und erstellt einen personalisierten Server-Guide via KI."""

    def __init__(self, bot: commands.Bot):
        self.bot = bot

    async def cog_load(self):
        await self._restore_persistent_views()
        log.info("AI Onboarding geladen (persistente Start-Buttons aktiv).")

    # ---------- Persistence ----------
    async def _persist_view(
        self, message_id: int, user_id: int | None, thread_id: int | None
    ) -> None:
        payload = {"user_id": user_id, "thread_id": thread_id}
        try:
            encoded = json.dumps(payload)
        except Exception:
            log.debug("Konnte View-Payload nicht serialisieren", exc_info=True)
            return
        try:
            await service_db.execute_async(
                "INSERT OR REPLACE INTO kv_store (ns, k, v) VALUES (?, ?, ?)",
                (NS_PERSIST_VIEWS, str(message_id), encoded),
            )
        except Exception:
            log.exception("Konnte persistente View nicht speichern (message_id=%s)", message_id)

    async def _clear_persisted_view(self, message_id: int) -> None:
        try:
            await service_db.execute_async(
                "DELETE FROM kv_store WHERE ns = ? AND k = ?",
                (NS_PERSIST_VIEWS, str(message_id)),
            )
        except Exception:
            log.debug(
                "Persistente View konnte nicht entfernt werden (message_id=%s)",
                message_id,
                exc_info=True,
            )

    async def _restore_persistent_views(self) -> None:
        try:
            # query_all_async is preferred to avoid blocking the event loop
            rows = await service_db.query_all_async(
                "SELECT k, v FROM kv_store WHERE ns = ?",
                (NS_PERSIST_VIEWS,),
            )
        except Exception:
            log.exception("Persistente AI-Onboarding-Views konnten nicht geladen werden")
            return

        restored = 0
        for row in rows:
            try:
                msg_id = int(row["k"] if isinstance(row, dict) else row[0])
                data_raw = row["v"] if isinstance(row, dict) else row[1]
                data = json.loads(data_raw)
            except Exception:
                await service_db.execute_async(
                    "DELETE FROM kv_store WHERE ns = ? AND k = ?",
                    (NS_PERSIST_VIEWS, str(row[0]) if row else "0"),
                )
                continue

            view = StartOnboardingView(
                self,
                allowed_user_id=data.get("user_id"),
                thread_id=data.get("thread_id"),
                message_id=msg_id,
            )
            try:
                self.bot.add_view(view, message_id=msg_id)
                restored += 1
            except Exception:
                log.debug(
                    "Persistente AI-Onboarding-View konnte nicht registriert werden (message_id=%s)",
                    msg_id,
                )
                await service_db.execute_async(
                    "DELETE FROM kv_store WHERE ns = ? AND k = ?", (NS_PERSIST_VIEWS, str(msg_id))
                )
        if restored:
            log.info("%s AI-Onboarding-Views nach Neustart reaktiviert", restored)

    # ---------- LLM ----------
    async def generate_personalized_text(
        self,
        *,
        answers: UserAnswers,
        user: discord.abc.User,
    ) -> tuple[str, dict[str, Any]]:
        meta: dict[str, Any] = {}
        role_context = _build_role_context_block(user, answers)

        prompt = dedent(
            f"""
            Kontext:
            {SERVER_CONTEXT}

            User:
            - Name: {getattr(user, "display_name", getattr(user, "name", "Nutzer"))}
            {role_context}
            {answers.as_prompt_block()}

            Form:
            - Ausgabe mit 3 Blöcken:
              1) Begrüßung (1–2 Sätze)
              2) "Kurz für dich" als 3–4 Bullets (je Bullet max. 1 Kanal)
              3) "Nächste Schritte" als 2–3 nummerierte Punkte
            - Keine doppelten Einleitungen, kein Fließtext mit vielen Kanälen.
            - Maximal 4 Kanäle insgesamt, nur aus dem Kontext.
            """
        ).strip()

        system_prompt = SYSTEM_PROMPT
        ai = getattr(self.bot, "get_cog", lambda name: None)("AIConnector")

        if ai:
            text, meta_resp = await ai.generate_text(
                provider="gemini",
                prompt=prompt,
                system_prompt=system_prompt,
                model=os.getenv("DEADLOCK_GEMINI_MODEL", "gemini-2.0-flash"),
                max_output_tokens=MAX_OUTPUT_TOKENS,
                temperature=0.45,
            )
            meta.update(meta_resp)
            if text:
                return text, meta

            text, meta_resp = await ai.generate_text(
                provider="openai",
                prompt=prompt,
                system_prompt=system_prompt,
                model=PRIMARY_MODEL,
                max_output_tokens=MAX_OUTPUT_TOKENS,
                temperature=0.45,
            )
            meta.update(meta_resp)
            if text:
                return text, meta

        fallback = (
            "Hey, willkommen auf dem Server! Basierend auf deinen Antworten:\n\n"
            f"- Interessen: {answers.interests or '-'}\n"
            f"- Erwartungen: {answers.expectations or '-'}\n"
            f"- Stil: {answers.style or 'locker'}\n\n"
            "Starte gern mit #spieler-suche, schau im Sprachkanal-Panel vorbei und gönn dir einen Blick in #ankündigungen. "
            "Für Fragen: /faq. Steam kannst du mit /account_verknüpfen koppeln. Viel Spaß! :)"
        )
        meta.setdefault("provider", "fallback")
        meta.setdefault("error", "no_ai_available")
        return fallback, meta

    # ---------- Public API ----------
    async def start_in_channel(
        self, channel: discord.abc.Messageable, member: discord.Member
    ) -> bool:
        """Postet den Start-Button in einen Thread/Channel und registriert Persistenz."""
        try:
            embed = discord.Embed(
                title="Willkommen! 🎉",
                description=(
                    "Lass uns kurz herausfinden, was du suchst – dann bekommst du eine auf dich zugeschnittene Tour.\n"
                    "Klick auf **Los geht's**, beantworte 2-3 Fragen und erhalte direkt Vorschläge, die zu dir passen."
                ),
                colour=discord.Colour.blue(),
            )
            view = StartOnboardingView(
                self,
                allowed_user_id=member.id,
                thread_id=getattr(channel, "id", None),
            )
            msg = await channel.send(embed=embed, view=view)
            view.message_id = msg.id
            # Persistenz für Reboots
            self.bot.add_view(view, message_id=msg.id)
            if not privacy.is_opted_out(member.id):
                await self._persist_view(msg.id, member.id, getattr(channel, "id", None))
            return True
        except Exception:
            log.exception("AI Onboarding konnte nicht gestartet werden")
            return False

    async def _log_session(
        self,
        *,
        user_id: int,
        thread_id: int | None,
        answers: UserAnswers,
        llm_meta: dict[str, Any],
    ) -> None:
        if privacy.is_opted_out(user_id):
            return
        try:
            payload = {
                "user_id": user_id,
                "thread_id": thread_id,
                "answers": {
                    "interests": answers.interests,
                    "expectations": answers.expectations,
                    "style": answers.style,
                },
                "llm": llm_meta,
            }
            encoded = json.dumps(payload)
            await service_db.execute_async(
                "INSERT OR REPLACE INTO kv_store (ns, k, v) VALUES (?, ?, ?)",
                (NS_SESSION_LOG, str(user_id), encoded),
            )
        except Exception:
            log.debug(
                "Session-Log konnte nicht gespeichert werden (user=%s)",
                user_id,
                exc_info=True,
            )


async def setup(bot: commands.Bot):
    await bot.add_cog(AIOnboarding(bot))
