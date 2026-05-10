"""
FAQ Chat Bot - Privater Chat für FAQ mit MiniMax.
"""

from __future__ import annotations

import asyncio
import logging
import os
from datetime import datetime, timedelta
from pathlib import Path

import discord
from discord import app_commands
from discord.ext import commands

from service import db as service_db

log = logging.getLogger(__name__)

# --- Konfiguration ---

PANEL_CHANNEL_ID = int(os.getenv("FAQ_PANEL_CHANNEL", "1491953161747955853"))
FAQ_CATEGORY_ID = int(os.getenv("FAQ_CATEGORY_ID", "1310153243795390475"))
TICKET_AUTO_HELP_CATEGORY_ID = int(os.getenv("TICKET_AUTO_HELP_CATEGORY_ID", "1459628097145147645"))
LOG_CHANNEL_ID = int(os.getenv("FAQ_LOG_CHANNEL", "1374364800817303632"))
SESSION_TIMEOUT_HOURS = int(os.getenv("FAQ_SESSION_HOURS", "24"))

PRIMARY_MODEL = os.getenv("MINIMAX_MODEL", "MiniMax-Text-01")
PRIMARY_PROVIDER = "minimax"
MAX_OUTPUT_TOKENS = int(os.getenv("FAQ_MAX_OUTPUT_TOKENS", "1500") or "1500")

DEFAULT_DOCS_PATH = Path(__file__).parent.parent / "docs"
DOCS_PATH = Path(os.getenv("FAQ_DOCS_PATH", str(DEFAULT_DOCS_PATH)))

PANEL_KV_NS = "faq_chat:panel"

# --- Doku-Loader ---


def _load_docs() -> str:
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
        "Nutze diese als Wissensbasis. Erfinde keine Informationen.\n"
    )
    return header + "\n".join(context_parts)


DOCS_CONTEXT = _load_docs()

# --- System Prompt ---


SYSTEM_PROMPT = """
Du bist ein hilfreicher, aber strikt eingeschränkter FAQ-Assistent.

SICHERHEITSREGELN (Pflicht!):
- Du bist ein ASSISTENT, kein Admin. Du kennst nur die bereitgestellte Dokumentation.
- Teile NIEMALS interne Pfade, API-Keys, Tokens, Secrets, Datenbank-URLs oder Konfigurationsdetails.
- Erfinde keine Server-Strukturen, Rollen oder Kanäle die nicht in der Dokumentation stehen.
- Biete niemals an, Code zu ändern, Bots neu zu starten oder externe Systeme zu konfigurieren.
- Wenn ein User fragt wie etwas intern funktioniert: sage dass du keinen Zugriff darauf hast.
- Wenn ein User eine Aktion braucht die du nicht kannst: verweise auf Deutsche Deadlock Community.

ANTWORTVERHALTEN:
- Antworte ausschließlich auf Deutsch.
- Sei hilfreich aber präzise. Nutze Emojis sparsam.
- Bei Fragen ausserhalb deines Wissens: ehrlich sagen dass du keine Info dazu hast.
- Für Feedback: verweise auf das anonyme Feedback-Formular.
- Halte Antworten informativ aber nicht übermässig lang.

INVITE / ONBOARDING – SONDERREGEL:
Wenn jemand fragt warum er keinen Invite hat, Deadlock nicht herunterladen kann, keinen Zugang zum Beta-Kanal hat oder einen Channel nicht sieht:
Gehe sofort die Checkliste durch:
1. Onboarding vollständig abgeschlossen? Bei Frage 4 "Ich brauche einen Invite / Betazugang" gewählt?
2. Rollen in #customize ausgewählt?
3. /betainvite im #beta-zugang verwendet?
4. Steam-Account hat min. 5 € Kaufhistorie?
Nenne alle offenen Punkte direkt und klar. Wenn das Problem offensichtlich daran liegt, dass jemand das Onboarding nicht gelesen hat, darf der Bot das freundlich aber ohne Umschweife sagen – zum Beispiel: "Das Onboarding enthält den genauen Hinweis dazu – wer es liest, spart sich das Ticket."
""".strip()

# --- DB Helpers ---


async def _ensure_db_tables() -> None:
    async with service_db.transaction() as tx:
        tx.execute("""
            CREATE TABLE IF NOT EXISTS faq_chat_sessions(
              session_id TEXT PRIMARY KEY,
              user_id INTEGER NOT NULL,
              user_name TEXT,
              channel_id INTEGER NOT NULL,
              guild_id INTEGER NOT NULL,
              created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              expires_at DATETIME NOT NULL,
              status TEXT NOT NULL DEFAULT 'active',
              last_activity_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        """)
        tx.execute("""
            CREATE TABLE IF NOT EXISTS faq_chat_messages(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL,
              role TEXT NOT NULL,
              content TEXT NOT NULL,
              created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              FOREIGN KEY(session_id) REFERENCES faq_chat_sessions(session_id)
            )
        """)
        tx.execute("""
            CREATE INDEX IF NOT EXISTS idx_faq_messages_session
            ON faq_chat_messages(session_id, created_at)
        """)
        tx.execute("""
            CREATE INDEX IF NOT EXISTS idx_faq_sessions_expires
            ON faq_chat_sessions(expires_at, status)
        """)


async def _get_stored_panel_msg_id() -> int | None:
    row = await service_db.query_one_async(
        "SELECT v FROM kv_store WHERE ns = ? AND k = ?",
        (PANEL_KV_NS, "panel_msg_id"),
    )
    if row:
        try:
            v = row[0] if not isinstance(row, dict) else row.get("v")
            return int(v)
        except (ValueError, TypeError):
            return None
    return None


async def _store_panel_msg_id(msg_id: int) -> None:
    await service_db.execute_async(
        "INSERT OR REPLACE INTO kv_store (ns, k, v) VALUES (?, ?, ?)",
        (PANEL_KV_NS, "panel_msg_id", str(msg_id)),
    )


async def _create_session(
    session_id: str,
    user_id: int,
    user_name: str,
    channel_id: int,
    guild_id: int,
) -> None:
    expires_at = datetime.utcnow() + timedelta(hours=SESSION_TIMEOUT_HOURS)
    async with service_db.transaction() as tx:
        tx.execute(
            """
            INSERT OR REPLACE INTO faq_chat_sessions
            (session_id, user_id, user_name, channel_id, guild_id, expires_at, status)
            VALUES (?, ?, ?, ?, ?, ?, 'active')
        """,
            (session_id, user_id, user_name, channel_id, guild_id, expires_at.isoformat()),
        )


async def _add_message(session_id: str, role: str, content: str) -> None:
    await service_db.execute_async(
        "INSERT INTO faq_chat_messages (session_id, role, content) VALUES (?, ?, ?)",
        (session_id, role, content),
    )
    await service_db.execute_async(
        "UPDATE faq_chat_sessions SET last_activity_at = ? WHERE session_id = ?",
        (datetime.utcnow().isoformat(), session_id),
    )


async def _get_session_messages(session_id: str) -> list[dict[str, str]]:
    rows = await service_db.query_all_async(
        "SELECT role, content FROM faq_chat_messages WHERE session_id = ? ORDER BY created_at",
        (session_id,),
    )
    return [{"role": r["role"], "content": r["content"]} for r in rows]


async def _get_expired_sessions() -> list:
    return await service_db.query_all_async(
        "SELECT session_id, channel_id, guild_id FROM faq_chat_sessions "
        "WHERE expires_at < ? AND status = 'active'",
        (datetime.utcnow().isoformat(),),
    )


async def _close_session(session_id: str) -> None:
    await service_db.execute_async(
        "UPDATE faq_chat_sessions SET status = 'closed' WHERE session_id = ?",
        (session_id,),
    )


# --- UI ---


class FAQPanelView(discord.ui.View):
    def __init__(self, cog: FAQChat) -> None:
        super().__init__(timeout=None)
        self.cog = cog

    @discord.ui.button(
        label="Frage stellen",
        style=discord.ButtonStyle.primary,
        emoji="💬",
        custom_id="faq_chat:start",
    )
    async def callback(self, interaction: discord.Interaction, button: discord.ui.Button) -> None:
        log.info("FAQ Panel button clicked by %s", interaction.user)
        await self.cog._on_panel_click(interaction)


class FAQChatView(discord.ui.View):
    def __init__(self, cog: FAQChat, session_id: str) -> None:
        super().__init__(timeout=None)
        self.cog = cog
        self.session_id = session_id

    @discord.ui.button(
        label="Chat beenden",
        style=discord.ButtonStyle.secondary,
        emoji="🛑",
        custom_id="faq_chat:close",
    )
    async def callback(self, interaction: discord.Interaction, button: discord.ui.Button) -> None:
        log.info("FAQ Close button clicked by %s", interaction.user)
        await self.cog._on_close_click(interaction, self.session_id)


# --- FAQ Chat Cog ---


TICKET_AUTO_HELP_SYSTEM_PROMPT = """
Du bist ein automatischer Ticket-Helfer. Entscheide anhand der Dokumentation ob du die Frage des Users lösen kannst.

REGEL:
- Wenn du die Frage aus der Dokumentation klar und vollständig beantworten kannst: antworte direkt und hilfreich.
- Wenn die Frage außerhalb deines Wissens liegt, unklar ist, oder menschlichen Support erfordert: antworte NUR mit dem Token KEIN_TREFFER und nichts weiter.
- Erfinde keine Informationen. Wenn du dir nicht sicher bist: KEIN_TREFFER.
- Antworte auf Deutsch, kurz und direkt.
""".strip()


class FAQChat(commands.Cog):
    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot
        self._panel_message_id: int | None = None
        self._cleanup_task: asyncio.Task | None = None
        self._ticket_pending: set[int] = set()  # Channel-IDs wo wir auf erste User-Msg warten

    async def cog_load(self) -> None:
        log.info("FAQ Chat cog_load start")
        await _ensure_db_tables()
        # Channel-Daten erst verfügbar wenn bot ready ist
        asyncio.ensure_future(self._delayed_setup())
        self._cleanup_task = asyncio.create_task(self._cleanup_loop())
        log.info("FAQ Chat gestartet (setup verzögert bis ready)")

    async def _delayed_setup(self) -> None:
        """Wartet bis Bot ready ist, dann Panel einrichten."""
        await self.bot.wait_until_ready()
        log.info("FAQ: Bot ready, setup panel...")
        await self._restore_panel_view()
        await self._ensure_panel()
        log.info("FAQ: Setup complete (panel_msg=%s)", self._panel_message_id)

    async def cog_unload(self) -> None:
        if self._cleanup_task:
            self._cleanup_task.cancel()

    async def _restore_panel_view(self) -> None:
        """Versucht gespeichertes Panel wiederherzustellen."""
        stored = await _get_stored_panel_msg_id()
        if not stored:
            log.info("FAQ: kein stored panel_msg_id")
            return

        log.info("FAQ: trying to restore panel_msg_id=%s", stored)
        channel = self.bot.get_channel(PANEL_CHANNEL_ID)
        if not channel:
            log.warning("FAQ: panel channel %s nicht gefunden", PANEL_CHANNEL_ID)
            return

        try:
            msg = await channel.fetch_message(stored)
            log.info("FAQ: found message %s in channel", msg.id)
        except discord.NotFound:
            log.info("FAQ: stored message %s not found in channel", stored)
            return
        except discord.Forbidden:
            log.warning("FAQ: no permission to fetch message %s", stored)
            return

        self.bot.add_view(FAQPanelView(self), message_id=msg.id)
        self._panel_message_id = msg.id
        log.info("FAQ Panel View restored: %s", msg.id)

    def _build_panel_embed(self) -> discord.Embed:
        embed = discord.Embed(
            title="FAQ - Häufig gestellte Fragen",
            description=(
                "Stell eine Frage zum Server, zu Kanälen, Rollen oder Deadlock.\n"
                "Klicke auf den Button – **ein Bot** versucht deine Frage zu beantworten.\n"
                "Deine Frage geht **nicht** an die Community.\n\n"
                "⏱️ Chats werden nach 24 Stunden automatisch geschlossen."
            ),
            colour=discord.Colour.blurple(),
        )
        embed.set_footer(text="Deadlock Master Bot • FAQ Chat")
        return embed

    async def _ensure_panel(self) -> None:
        """Erstellt Panel falls keins existiert, aktualisiert es andernfalls."""
        channel = self.bot.get_channel(PANEL_CHANNEL_ID)
        if not channel:
            log.warning("FAQ: panel channel %s nicht gefunden", PANEL_CHANNEL_ID)
            return

        if self._panel_message_id:
            try:
                msg = await channel.fetch_message(self._panel_message_id)
                await msg.edit(embed=self._build_panel_embed())
                log.info("FAQ Panel aktualisiert: msg_id=%s", self._panel_message_id)
                return
            except (discord.NotFound, discord.Forbidden):
                log.info("FAQ: stored panel message nicht gefunden, erstelle neu")
                self._panel_message_id = None

        view = FAQPanelView(self)
        try:
            msg = await channel.send(embed=self._build_panel_embed(), view=view)
            await _store_panel_msg_id(msg.id)
            self._panel_message_id = msg.id
            log.info("FAQ Panel erstellt: msg_id=%s channel=%s", msg.id, channel.id)
        except discord.Forbidden:
            log.error("FAQ: keine rechte um panel zu erstellen in %s", channel.id)

    async def _create_faq_channel(
        self,
        guild: discord.Guild,
        user: discord.User,
    ) -> tuple[str, discord.TextChannel]:
        session_id = f"faq-{user.id}-{datetime.utcnow().strftime('%Y%m%d%H%M%S')}"

        overwrites = {
            guild.default_role: discord.PermissionOverwrite(view_channel=False),
            user: discord.PermissionOverwrite(
                view_channel=True,
                send_messages=True,
                read_messages=True,
                read_message_history=True,
            ),
            guild.me: discord.PermissionOverwrite(
                view_channel=True,
                send_messages=True,
                read_messages=True,
                read_message_history=True,
                manage_channels=True,
            ),
        }

        category = guild.get_channel(FAQ_CATEGORY_ID)
        if not isinstance(category, discord.CategoryChannel):
            log.warning("FAQ Category %s nicht gefunden", FAQ_CATEGORY_ID)
            category = None

        channel_name = f"faq-{user.name.lower().replace(' ', '-')}"
        try:
            if category:
                channel = await guild.create_text_channel(
                    name=channel_name,
                    category=category,
                    overwrites=overwrites,
                    reason="FAQ Chat",
                )
            else:
                channel = await guild.create_text_channel(
                    name=channel_name,
                    overwrites=overwrites,
                    reason="FAQ Chat",
                )
        except discord.Forbidden:
            log.error("FAQ: keine rechte channel zu erstellen in guild %s", guild.id)
            raise

        await _create_session(
            session_id=session_id,
            user_id=user.id,
            user_name=str(user),
            channel_id=channel.id,
            guild_id=guild.id,
        )
        return session_id, channel

    async def _on_panel_click(self, interaction: discord.Interaction) -> None:
        """Behandelt den Klick auf den Panel-Button."""
        try:
            if interaction.response.is_done():
                log.warning("FAQ: interaction response already done")
                await interaction.followup.send(
                    "Bitte versuche es erneut.",
                    ephemeral=True,
                )
                return

            await interaction.response.defer(ephemeral=True)
        except discord.InteractionResponded:
            pass  # Another handler already responded to this interaction.
        except Exception as exc:
            log.exception("FAQ: defer failed: %s", exc)
            try:
                await interaction.followup.send(f"❌ Fehler bei defer: {exc}", ephemeral=True)
            except Exception:
                pass  # best-effort error report; ignore send failures
            return

        user = interaction.user
        guild = interaction.guild

        if not guild:
            await interaction.followup.send(
                "❌ Das funktioniert nur auf dem Server.", ephemeral=True
            )
            return

        # Check auf bestehenden Chat
        row = await service_db.query_one_async(
            "SELECT session_id, channel_id FROM faq_chat_sessions "
            "WHERE user_id = ? AND status = 'active'",
            (user.id,),
        )
        if row:
            channel_id = row[1] if not isinstance(row, dict) else row.get("channel_id")
            existing_channel = guild.get_channel(channel_id)
            if existing_channel:
                await interaction.followup.send(
                    f"❌ Du hast bereits einen aktiven Chat: {existing_channel.mention}",
                    ephemeral=True,
                )
                return

        try:
            session_id, channel = await self._create_faq_channel(guild, user)
        except Exception as exc:
            log.exception("FAQ: channel creation failed: %s", exc)
            await interaction.followup.send(
                f"❌ Konnte keinen Chat erstellen: {exc}", ephemeral=True
            )
            return

        view = FAQChatView(self, session_id)
        welcome = (
            f"👋 **{user.name}**, willkommen zum FAQ-Chat!\n\n"
            "Stell mir Fragen zum Server, zu Kanälen, Rollen, Bots oder Deadlock.\n"
            "Ich kann mich an unsere Unterhaltung erinnern - du kannst auch Rückfragen stellen.\n\n"
            "⏱️ Dieser Chat wird nach 24 Stunden automatisch geschlossen.\n"
            "🛑 Du kannst den Chat jederzeit mit dem Button unten beenden."
        )
        try:
            await channel.send(welcome, view=view)
        except discord.Forbidden:
            log.error("FAQ: konnte willkommensnachricht nicht senden in %s", channel.id)

        await interaction.followup.send(
            f"✅ Dein FAQ-Chat wurde erstellt: {channel.mention}\n\nStell deine Frage(n) dort.",
            ephemeral=True,
        )

    async def _on_close_click(self, interaction: discord.Interaction, session_id: str) -> None:
        try:
            if interaction.response.is_done():
                await interaction.followup.send("Bitte versuche es erneut.", ephemeral=True)
                return
            await interaction.response.defer(ephemeral=True)
        except discord.InteractionResponded:
            pass  # Another handler already responded to this interaction.
        except Exception as exc:
            log.exception("FAQ close defer failed")
            try:
                await interaction.followup.send(f"❌ Fehler: {exc}", ephemeral=True)
            except Exception:
                pass  # best-effort error report; ignore send failures
            return

        row = await service_db.query_one_async(
            "SELECT user_id, channel_id FROM faq_chat_sessions WHERE session_id = ? AND status = 'active'",
            (session_id,),
        )
        if not row:
            await interaction.followup.send("❌ Session nicht gefunden.", ephemeral=True)
            return

        user_id = row[0] if not isinstance(row, dict) else row.get("user_id")
        if interaction.user.id != user_id:
            await interaction.followup.send("❌ Das ist nicht dein Chat.", ephemeral=True)
            return

        await _close_session(session_id)

        channel_id = row[1] if not isinstance(row, dict) else row.get("channel_id")
        channel = interaction.guild.get_channel(channel_id)
        if channel and isinstance(channel, discord.TextChannel):
            try:
                await channel.send("🛑 Chat beendet.")
                await channel.edit(archived=True)
            except discord.Forbidden:
                pass  # Archive/send failure is non-critical after manual chat closure.

        await interaction.followup.send("✅ Chat beendet.", ephemeral=True)

    async def _handle_chat_message(self, message: discord.Message) -> None:
        if message.author.bot:
            return
        if not isinstance(message.channel, discord.TextChannel):
            return

        row = await service_db.query_one_async(
            "SELECT session_id, user_id FROM faq_chat_sessions "
            "WHERE channel_id = ? AND status = 'active'",
            (message.channel.id,),
        )
        if not row:
            return

        session_id = row[0] if not isinstance(row, dict) else row.get("session_id")
        user_id = row[1] if not isinstance(row, dict) else row.get("user_id")

        if message.author.id != user_id:
            return

        question = message.content.strip()
        if not question:
            return

        await _add_message(session_id, "user", question)
        await self._log_qa(user_id, str(message.author), question, message.channel.id)

        async with message.channel.typing():
            answer, model = await self._generate_answer(session_id, question)

        await _add_message(session_id, "assistant", answer)

        try:
            await message.reply(answer, suppress_embeds=True)
        except discord.HTTPException:
            await message.channel.send(f"{message.author.mention}: {answer}", suppress_embeds=True)

    @commands.Cog.listener("on_message")
    async def on_message(self, message: discord.Message) -> None:
        if message.guild is None:
            return
        await self._handle_chat_message(message)
        await self._handle_ticket_auto_help(message)

    @commands.Cog.listener("on_guild_channel_create")
    async def on_guild_channel_create(self, channel: discord.abc.GuildChannel) -> None:
        if not isinstance(channel, discord.TextChannel):
            return
        if not channel.category_id or channel.category_id != TICKET_AUTO_HELP_CATEGORY_ID:
            return
        # Nur echte Ticket-Channels (Name beginnt mit "ticket-")
        if not channel.name.startswith("ticket-"):
            return
        log.info("FAQ: neues Ticket erkannt: #%s (%s) – warte auf erste User-Nachricht", channel.name, channel.id)
        self._ticket_pending.add(channel.id)
        # Timeout: nach 10 Minuten aus pending entfernen falls kein User schreibt
        asyncio.ensure_future(self._ticket_pending_timeout(channel.id))

    async def _ticket_pending_timeout(self, channel_id: int) -> None:
        await asyncio.sleep(600)
        self._ticket_pending.discard(channel_id)

    async def _handle_ticket_auto_help(self, message: discord.Message) -> None:
        if message.author.bot:
            return
        if message.channel.id not in self._ticket_pending:
            return
        if not isinstance(message.channel, discord.TextChannel):
            return

        self._ticket_pending.discard(message.channel.id)
        problem = message.content.strip()
        if not problem:
            return

        log.info("FAQ auto-help: Ticket #%s – analysiere '%s...'", message.channel.name, problem[:60])

        async with message.channel.typing():
            answer = await self._ticket_auto_answer(problem)

        if answer is None:
            log.info("FAQ auto-help: kein Treffer für Ticket #%s – schweige", message.channel.name)
            return

        log.info("FAQ auto-help: Antwort für Ticket #%s gepostet", message.channel.name)
        try:
            await message.reply(answer, suppress_embeds=True)
        except discord.HTTPException:
            await message.channel.send(answer, suppress_embeds=True)

    async def _ticket_auto_answer(self, problem: str) -> str | None:
        """Gibt eine Antwort zurück wenn der Bot helfen kann, sonst None."""
        ai = getattr(self.bot, "get_cog", lambda n: None)("AIConnector")
        if not ai:
            return None

        full_prompt = (
            f"Dokumentation:\n{DOCS_CONTEXT}\n\n"
            f"Ticket-Inhalt:\n{problem.strip()}"
        )

        try:
            answer_text, _ = await ai.generate_text(
                provider=PRIMARY_PROVIDER,
                prompt=full_prompt,
                system_prompt=TICKET_AUTO_HELP_SYSTEM_PROMPT,
                model=PRIMARY_MODEL,
                max_output_tokens=MAX_OUTPUT_TOKENS,
                temperature=0.2,
            )
        except Exception as exc:
            log.warning("FAQ auto-help: AI-Fehler: %s", exc)
            return None

        if not answer_text:
            return None
        answer_text = answer_text.strip()
        if "KEIN_TREFFER" in answer_text:
            return None
        return answer_text

    async def _generate_answer(self, session_id: str, new_question: str) -> tuple[str, str | None]:
        ai = getattr(self.bot, "get_cog", lambda n: None)("AIConnector")
        if not ai:
            return "Der FAQ-Service ist aktuell nicht verfügbar.", None

        messages = await _get_session_messages(session_id)
        conversation_context = ""
        if messages:
            lines = []
            for msg in messages[-10:]:
                role_label = "User" if msg["role"] == "user" else "Assistent"
                lines.append(f"{role_label}: {msg['content']}")
            conversation_context = "\n".join(lines)

        patchnote_context = ""
        try:
            from service import changelogs

            patchnote_context = changelogs.get_context_for_question(new_question)
        except Exception:
            pass  # Patchnote enrichment is optional for FAQ answers.

        context_parts = [DOCS_CONTEXT]
        if patchnote_context:
            context_parts.append(f"Patchnotes:\n{patchnote_context}")
        if conversation_context:
            context_parts.append(f"Bisherige Konversation:\n{conversation_context}")

        full_prompt = (
            "Dokumentation:\n"
            + "\n\n---\n\n".join(context_parts)
            + f"\n\nNeue Frage:\n{new_question.strip()}"
        )

        answer_text, meta = await ai.generate_text(
            provider=PRIMARY_PROVIDER,
            prompt=full_prompt,
            system_prompt=SYSTEM_PROMPT,
            model=PRIMARY_MODEL,
            max_output_tokens=MAX_OUTPUT_TOKENS,
            temperature=0.3,
        )
        model = meta.get("model")
        if not answer_text:
            return "Ich konnte keine Antwort generieren.", model
        return answer_text.strip(), model

    async def _log_qa(self, user_id: int, username: str, question: str, channel_id: int) -> None:
        channel = self.bot.get_channel(LOG_CHANNEL_ID)
        if not channel:
            return
        embed = discord.Embed(
            title="FAQ Chat",
            color=discord.Colour.blue(),
            timestamp=datetime.utcnow(),
        )
        embed.add_field(name="User", value=f"{username} ({user_id})", inline=False)
        embed.add_field(name="Channel", value=f"<#{channel_id}>", inline=False)
        embed.add_field(name="Frage", value=question[:1024], inline=False)
        try:
            await channel.send(embed=embed)
        except Exception:
            pass  # FAQ logging is best-effort and should not break the request flow.

    async def _cleanup_loop(self) -> None:
        while True:
            try:
                await asyncio.sleep(3600)
                await self._cleanup_expired()
            except asyncio.CancelledError:
                break
            except Exception:
                log.exception("Fehler beim FAQ Cleanup")

    async def _cleanup_expired(self) -> None:
        expired = await _get_expired_sessions()
        if not expired:
            return
        log.info("FAQ: %d abgelaufene Sessions", len(expired))
        for row in expired:
            session_id = row[0] if not isinstance(row, dict) else row.get("session_id")
            channel_id = row[1] if not isinstance(row, dict) else row.get("channel_id")
            guild_id = row[2] if not isinstance(row, dict) else row.get("guild_id")
            await _close_session(session_id)
            guild = self.bot.get_guild(guild_id)
            if guild:
                channel = guild.get_channel(channel_id)
                if channel and isinstance(channel, discord.TextChannel):
                    try:
                        await channel.send("⏱️ Chat wurde automatisch geschlossen (24h Timeout).")
                        await channel.edit(archived=True)
                    except discord.Forbidden:
                        pass  # Archive/send failure is non-critical during timeout cleanup.

    @app_commands.command(name="faqpanel", description="Erstellt das FAQ Panel (Admin)")
    @app_commands.default_permissions(administrator=True)
    async def faqpanel(self, interaction: discord.Interaction) -> None:
        await interaction.response.defer(ephemeral=True)

        if self._panel_message_id:
            channel = self.bot.get_channel(PANEL_CHANNEL_ID)
            if channel:
                try:
                    msg = await channel.fetch_message(self._panel_message_id)
                    await interaction.followup.send(
                        f"✅ FAQ Panel existiert bereits: {msg.jump_url}",
                        ephemeral=True,
                    )
                    return
                except (discord.NotFound, discord.Forbidden):
                    pass

        await self._ensure_panel()
        if self._panel_message_id:
            channel = self.bot.get_channel(PANEL_CHANNEL_ID)
            if channel:
                try:
                    msg = await channel.fetch_message(self._panel_message_id)
                    await interaction.followup.send(
                        f"✅ FAQ Panel wurde erstellt: {msg.jump_url}",
                        ephemeral=True,
                    )
                except (discord.NotFound, discord.Forbidden):
                    await interaction.followup.send("✅ FAQ Panel wurde erstellt.", ephemeral=True)
        else:
            await interaction.followup.send(
                f"❌ Konnte Panel nicht erstellen. Channel {PANEL_CHANNEL_ID} prüfen.",
                ephemeral=True,
            )


async def setup(bot: commands.Bot) -> None:
    faq_cog = FAQChat(bot)
    await bot.add_cog(faq_cog)
    try:
        bot.tree.add_command(faq_cog.faqpanel)
    except app_commands.CommandAlreadyRegistered:
        pass  # Command registration is idempotent; duplicates are acceptable.
