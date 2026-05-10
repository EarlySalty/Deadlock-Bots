"""
MiniMax-gestützter FAQ Bot - Chat-basiert mit Thread-Interface.

Sicherheitsrichtlinien:
- User wird als untrusted behandelt
- Kein Zugriff auf Infisical/Secrets
- Keine Code-Änderungen möglich (read-only)
- Nur Antworten / Helfen basierend auf Dokumentation
"""

from __future__ import annotations

import asyncio
import logging
import os
from datetime import datetime
from pathlib import Path
from typing import Any

import discord
from discord import app_commands
from discord.ext import commands

log = logging.getLogger(__name__)

# --- Konfiguration ------------------------------------------------------------

PRIMARY_MODEL = os.getenv("MINIMAX_MODEL", "MiniMax-Text-01")
PRIMARY_PROVIDER = "minimax"
MAX_OUTPUT_TOKENS = int(os.getenv("DEADLOCK_FAQ_MAX_OUTPUT_TOKENS", "1500") or "1500")
LOG_CHANNEL_ID = int(os.getenv("DEADLOCK_FAQ_LOG_CHANNEL", "1374364800817303632"))

# Thread-Channel wo FAQ-Chats erstellt werden
FAQ_THREAD_CHANNEL_ID = int(os.getenv("DEADLOCK_FAQ_THREAD_CHANNEL", "1374364800817303632"))

# Docs-Pfad
DEFAULT_DOCS_PATH = Path(__file__).parent.parent / "docs"
DOCS_PATH = Path(os.getenv("DEADLOCK_FAQ_DOCS_PATH", str(DEFAULT_DOCS_PATH)))

# Session-Cookie in ms - wie lange ein Thread aktiv bleibt
SESSION_TIMEOUT_HOURS = int(os.getenv("DEADLOCK_FAQ_SESSION_HOURS", "24"))

# --- Doku-Loader --------------------------------------------------------------


def _load_docs() -> str:
    """Lädt alle .md Dateien aus dem Docs-Verzeichnis als Kontext."""
    if not DOCS_PATH.is_dir():
        log.warning("Docs-Pfad nicht gefunden: %s", DOCS_PATH)
        return ""

    context_parts = []
    for md_file in sorted(DOCS_PATH.glob("*.md")):
        try:
            content = md_file.read_text(encoding="utf-8")
            rel_path = md_file.relative_to(DOCS_PATH)
            context_parts.append(f"\n\n=== Dokument: {rel_path} ===\n{content}")
        except Exception as exc:
            log.warning("Konnte %s nicht lesen: %s", md_file, exc)

    if not context_parts:
        return ""

    header = (
        "Du hast Zugriff auf folgende Server-Dokumentation. "
        "Nutze diese als Wissensbasis. Erfinde keine Informationen - wenn du dir unsicher bist, sage es.\n"
    )
    return header + "\n".join(context_parts)


DOCS_CONTEXT = _load_docs()

# --- System Prompt (Security!) ------------------------------------------------


SYSTEM_PROMPT = """
Du bist ein hilfreicher, aber strikt eingeschränkter FAQ-Assistent für einen Discord-Server.

SICHERHEITSREGELN (Pflicht!):
- Du bist ein ASSISTENT, kein Admin. Du kennst nur die bereitgestellte Dokumentation.
- Teile NIEMALS interne Pfade, API-Keys, Tokens, Secrets, Datenbank-URLs oder Konfigurationsdetails.
- Erfinde keine Server-Strukturen, Rollen oder Kanäle die nicht in der Dokumentation stehen.
- Biete niemals an, Code zu ändern, Bots neu zu starten oder externe Systeme zu konfigurieren.
- Wenn ein User fragt wie etwas intern funktioniert (Tokens, APIs, Secrets): sage dass du keinen Zugriff darauf hast.
- Wenn ein User eine Aktion braucht die du nicht支撑 kannst: verweise auf Deutsche Deadlock Community.

ANTWORTVERHALTEN:
- Antworte ausschließlich auf Deutsch.
- Sei hilfreich aber präzise. Lies die Dokumentation und beantworte basierend darauf.
- Bei Fragen ausserhalb deines Wissens: ehrlich sagen dass du dazu keine Info hast.
- Nutze Emojis sparsam und passend.
- Halte Antworten informativ aber nicht übermässig lang.
- Für Feedback: verweise auf das anonyme Feedback-Formular im Feedback Hub.
- Für Match-Coaching, TempVoice, Spieler-Suche: verweise auf die entsprechen Kanäle/Befehle.
""".strip()

# --- Session Management --------------------------------------------------------


class FAQSession:
    """Hält Kontext für einen FAQ-Chat."""

    def __init__(
        self,
        user_id: int,
        thread_id: int,
        created_at: datetime,
    ) -> None:
        self.user_id = user_id
        self.thread_id = thread_id
        self.created_at = created_at
        self.messages: list[dict[str, str]] = []  # [{"role": "user"|"assistant", "content": ...}]
        self.last_activity = datetime.utcnow()

    def add_user_message(self, content: str) -> None:
        self.messages.append({"role": "user", "content": content})
        self.last_activity = datetime.utcnow()

    def add_assistant_message(self, content: str) -> None:
        self.messages.append({"role": "assistant", "content": content})
        self.last_activity = datetime.utcnow()

    def get_conversation_context(self) -> str:
        """Gibt die Konversation als String zurück für den AI-Prompt."""
        if not self.messages:
            return ""
        lines = []
        for msg in self.messages[-10:]:  # Letzte 10 Nachrichten
            role_label = "User" if msg["role"] == "user" else "Assistent"
            lines.append(f"{role_label}: {msg['content']}")
        return "\n".join(lines)


# --- Cog ----------------------------------------------------------------------


class ServerFAQ(commands.Cog):
    """Chat-basierter FAQ Bot mit MiniMax."""

    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot
        # session_id (int) -> FAQSession
        self._sessions: dict[int, FAQSession] = {}
        # thread_id -> session_id
        self._thread_to_session: dict[int, int] = {}
        # user_id -> session_id (nur ein aktiver Chat pro User)
        self._user_to_session: dict[int, int] = {}
        self._lock = asyncio.Lock()

    # ---- Session Management ----

    async def _get_or_create_session(
        self,
        user_id: int,
        thread_id: int,
    ) -> FAQSession:
        async with self._lock:
            # Bestehende Session für User oder Thread?
            if user_id in self._user_to_session:
                sid = self._user_to_session[user_id]
                session = self._sessions.get(sid)
                if session:
                    return session
            if thread_id in self._thread_to_session:
                sid = self._thread_to_session[thread_id]
                session = self._sessions.get(sid)
                if session:
                    return session

            # Neue Session
            session = FAQSession(
                user_id=user_id,
                thread_id=thread_id,
                created_at=datetime.utcnow(),
            )
            self._sessions[id(session)] = session
            self._thread_to_session[thread_id] = id(session)
            self._user_to_session[user_id] = id(session)
            return session

    async def _cleanup_old_sessions(self) -> None:
        """Entfernt Sessions älter als SESSION_TIMEOUT_HOURS."""
        async with self._lock:
            now = datetime.utcnow()
            expired = [
                sid
                for sid, s in self._sessions.items()
                if (now - s.last_activity).total_seconds() > SESSION_TIMEOUT_HOURS * 3600
            ]
            for sid in expired:
                session = self._sessions.pop(sid, None)
                if session:
                    self._thread_to_session.pop(session.thread_id, None)
                    self._user_to_session.pop(session.user_id, None)

    # ---- Thread Creation ----

    async def _create_faq_thread(
        self,
        interaction: discord.Interaction,
        user: discord.User,
    ) -> discord.Thread:
        """Erstellt einen neuen FAQ-Thread für den User."""
        guild = interaction.guild
        if not guild:
            raise ValueError("Kein Guild")

        channel = guild.get_channel(FAQ_THREAD_CHANNEL_ID)
        if not channel:
            raise ValueError(f"Thread-Channel {FAQ_THREAD_CHANNEL_ID} nicht gefunden")

        thread_name = f"FAQ-{user.name}-{datetime.utcnow().strftime('%d.%m %H:%M')}"
        thread = await channel.create_thread(
            name=thread_name,
            type=discord.ChannelType.public_thread,
            invitable=False,
        )
        # User zum Thread hinzufügen
        await thread.add_user(user)
        return thread

    # ---- AI Answer Generation ----

    async def _generate_answer(
        self,
        session: FAQSession,
        new_question: str,
    ) -> tuple[str, dict[str, Any]]:
        """Generiert Antwort mit Kontext der bisherigen Konversation."""
        metadata: dict[str, Any] = {}

        ai = getattr(self.bot, "get_cog", lambda n: None)("AIConnector")
        if not ai:
            return (
                "Der FAQ-Service ist aktuell nicht verfügbar. Bitte versuche es später erneut.",
                metadata,
            )

        conversation_context = session.get_conversation_context()

        # Patchnotes-Kontext falls relevant
        patchnote_context = ""
        try:
            from service import changelogs

            patchnote_context = changelogs.get_context_for_question(new_question)
        except Exception:
            pass  # Patchnote enrichment is optional for FAQ answers.

        # Prompt zusammensetzen
        context_parts = [DOCS_CONTEXT]
        if patchnote_context:
            context_parts.append(f"Patchnotes-Kontext:\n{patchnote_context}")

        if conversation_context:
            context_parts.append(f"Bisherige Konversation:\n{conversation_context}")

        full_prompt = (
            "Dokumentation:\n"
            + "\n\n---\n\n".join(context_parts)
            + f"\n\nNeue Frage vom User:\n{new_question.strip()}"
        )

        answer_text, meta = await ai.generate_text(
            provider=PRIMARY_PROVIDER,
            prompt=full_prompt,
            system_prompt=SYSTEM_PROMPT,
            model=PRIMARY_MODEL,
            max_output_tokens=MAX_OUTPUT_TOKENS,
            temperature=0.3,
        )
        metadata.update(meta)

        if not answer_text:
            return "Ich konnte keine Antwort generieren. Bitte versuche es später erneut.", metadata

        return answer_text.strip(), metadata

    # ---- Message Handler ----

    async def _handle_thread_message(
        self,
        message: discord.Message,
    ) -> None:
        """Verarbeitet eine Nachricht in einem FAQ-Thread."""
        # Bot-Nachrichten ignorieren
        if message.author.id == self.bot.user.id:
            return

        # Nur in Threads die wir kennen
        if message.channel.id not in self._thread_to_session:
            return

        session_id = self._thread_to_session[message.channel.id]
        session = self._sessions.get(session_id)
        if not session:
            return

        # Nur der Thread-Owner darf chatten
        if message.author.id != session.user_id:
            return

        question = message.content.strip()
        if not question:
            return

        # Thinking-Message senden
        async with message.channel.typing():
            session.add_user_message(question)
            answer, metadata = await self._generate_answer(session, question)

        session.add_assistant_message(answer)

        # Logging
        await self._log_qa(
            user_id=message.author.id,
            username=str(message.author),
            question=question,
            answer=answer,
            model=metadata.get("model"),
        )

        # Antwort ephemeral senden (nur User sieht sie)
        try:
            await message.reply(answer, suppress_embeds=True)
        except discord.HTTPException:
            # Fallback: direkt antworten wenn reply nicht geht
            await message.channel.send(
                f"{message.author.mention}: {answer}",
                suppress_embeds=True,
            )

    # ---- Logging ----

    async def _log_qa(
        self,
        user_id: int,
        username: str,
        question: str,
        answer: str,
        model: str | None,
    ) -> None:
        """Loggt Q&A in den dedizierten Channel."""
        channel = self.bot.get_channel(LOG_CHANNEL_ID)
        if not channel:
            log.warning("Log-Channel %d nicht gefunden", LOG_CHANNEL_ID)
            return

        embed = discord.Embed(
            title="FAQ Chat",
            color=discord.Colour.blue(),
            timestamp=datetime.utcnow(),
        )
        embed.add_field(name="User", value=f"{username} ({user_id})", inline=False)
        embed.add_field(name="Frage", value=question[:1024], inline=False)
        embed.add_field(name="Antwort", value=answer[:1024] if answer else "(leer)", inline=False)
        if model:
            embed.set_footer(text=f"Model: {model}")

        try:
            await channel.send(embed=embed)
        except Exception as exc:
            log.error("Konnte Q&A nicht loggen: %s", exc)

    # ---- Discord Event ----

    @commands.Cog.listener("on_message")
    async def on_message(self, message: discord.Message) -> None:
        """Reagiert auf Nachrichten in FAQ-Threads."""
        if message.guild is None:
            return
        if not isinstance(message.channel, discord.Thread):
            return
        if message.author.bot:
            return

        # Periodic cleanup
        if message.id % 50 == 0:  # Alle 50 Nachrichten aufräumen
            asyncio.create_task(self._cleanup_old_sessions())

        await self._handle_thread_message(message)

    # ---- Commands ----

    @app_commands.command(
        name="faq",
        description="Startet einen FAQ-Chat mit dem Server-Assistenten.",
    )
    @app_commands.guild_only()
    async def faq(self, interaction: discord.Interaction) -> None:
        """Startet einen neuen FAQ-Chat für den User."""
        await interaction.response.defer(ephemeral=True)

        user = interaction.user

        try:
            thread = await self._create_faq_thread(interaction, user)
        except ValueError as exc:
            await interaction.followup.send(
                f"❌ Konnte keinen Chat erstellen: {exc}",
                ephemeral=True,
            )
            return
        except discord.HTTPException:
            await interaction.followup.send(
                "❌ Konnte keinen Chat erstellen. Bitte versuche es später erneut.",
                ephemeral=True,
            )
            return

        _session = await self._get_or_create_session(user.id, thread.id)

        # Willkommensnachricht im Thread
        welcome = (
            f"👋 **{user.name}**, willkommen beim FAQ-Chat!\n\n"
            "Stell mir Fragen zum Server, zu Kanälen, Rollen, Bots oder Deadlock.\n"
            "Ich kann mich an unsere bisherige Unterhaltung erinnern - du kannst also "
            "auch Rückfragen stellen.\n\n"
            "⏱️ Dieser Chat bleibt 24 Stunden aktiv."
        )

        try:
            await thread.send(welcome)
        except discord.HTTPException:
            pass  # Welcome message failure is non-critical after thread creation.

        await interaction.followup.send(
            f"✅ Dein FAQ-Chat wurde erstellt: {thread.mention}\n\n"
            "Klicke auf den Link und stell deine Frage(n) dort.",
            ephemeral=True,
        )

    @app_commands.command(
        name="faqclose",
        description="Beendet den aktuellen FAQ-Chat.",
    )
    @app_commands.guild_only()
    async def faqclose(self, interaction: discord.Interaction) -> None:
        """Beendet den FAQ-Chat des Users."""
        user_id = interaction.user.id
        channel = interaction.channel

        if not isinstance(channel, discord.Thread):
            await interaction.response.send_message(
                "❌ Dieser Befehl funktioniert nur in einem FAQ-Thread.",
                ephemeral=True,
            )
            return

        session_id = self._thread_to_session.get(channel.id)
        if not session_id:
            await interaction.response.send_message(
                "❌ Das ist kein aktiver FAQ-Thread.",
                ephemeral=True,
            )
            return

        session = self._sessions.get(session_id)
        if not session or session.user_id != user_id:
            await interaction.response.send_message(
                "❌ Das ist nicht dein FAQ-Thread.",
                ephemeral=True,
            )
            return

        async with self._lock:
            self._sessions.pop(session_id, None)
            self._thread_to_session.pop(channel.id, None)
            self._user_to_session.pop(user_id, None)

        try:
            await channel.send("🛑 Dieser FAQ-Chat wurde beendet.")
            await channel.edit(archived=True)
        except discord.HTTPException:
            pass  # Archive/send failure is non-critical after closing the session state.

        await interaction.response.send_message(
            "✅ Dein FAQ-Chat wurde beendet.",
            ephemeral=True,
        )


async def setup(bot: commands.Bot) -> None:
    faq_cog = ServerFAQ(bot)
    await bot.add_cog(faq_cog)

    try:
        bot.tree.add_command(faq_cog.faq)
    except app_commands.CommandAlreadyRegistered:
        bot.tree.remove_command("faq", type=discord.AppCommandType.chat_input)
        bot.tree.add_command(faq_cog.faq)

    try:
        bot.tree.add_command(faq_cog.faqclose)
    except app_commands.CommandAlreadyRegistered:
        bot.tree.remove_command("faqclose", type=discord.AppCommandType.chat_input)
        bot.tree.add_command(faq_cog.faqclose)
