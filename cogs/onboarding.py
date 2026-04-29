"""Statisches Multi-Step Onboarding – kein AI, kein Role-Gate, 9 klare Schritte."""

from __future__ import annotations

import logging
from typing import Any

import discord
from discord.ext import commands

from service.config import settings

log = logging.getLogger(__name__)

GUILD_ID = settings.guild_id
VERIFIED_ROLE_ID = settings.verified_role_id  # Rolle die nach Steam-Verifizierung vergeben wird
CONTENT_CREATOR_ROLE_ID = settings.content_creator_role_id

# Channel-IDs für klickbare Mentions in Embeds (<#ID>)
CH_LFG = 1376335502919335936  # #spieler-suche
CH_TEMPVOICE = 1371927143537315890  # #sprach-kanal-verwalten
CH_RULES = 1315684135175716975  # #regelwerk
CH_FEEDBACK = 1289721245281292291  # #feedback-kanal
CH_CLIPS = 1425215762460835931  # #clip-submission
CH_COACHING = 1357421075188813897  # #ich-brauch-einen-coach
CH_TICKET = None  # #ticket-eröffnen (Mention via Text)
CH_BETA = 1428745737323155679  # #beta-zugang
CH_STREAMER_INFO = 1374364800817303632  # #streamer-info (Beispiel)

try:
    from cogs.steam import steam_link_oauth as _steam_link_oauth  # type: ignore
except Exception:
    _steam_link_oauth = None  # type: ignore[assignment]
    log.info("Steam link OAuth module unavailable – onboarding link buttons will be degraded.")

if _steam_link_oauth is not None and not hasattr(_steam_link_oauth, "start_urls_for"):
    log.warning(
        "cogs.steam.steam_link_oauth is missing 'start_urls_for'; onboarding link buttons will be degraded."
    )
    _steam_link_oauth = None  # type: ignore[assignment]


# ---------------------------------------------------------------------------
# Schritt-Definitionen
# ---------------------------------------------------------------------------


def _c(channel_id: int) -> str:
    """Gibt einen klickbaren Channel-Mention zurück."""
    return f"<#{channel_id}>"


STEPS: list[dict] = [
    # ── 0 ─────────────────────────────────────────────────────────────────
    {
        "title": "Hey, willkommen auf dem Server! 👋",
        "description": (
            "Schön dass du dabei bist – wir sind die **Deutsche Deadlock Community**.\n\n"
            "Egal ob du gerade erst anfängst oder schon ein paar hundert Stunden drinhast, "
            "hier findest du Leute zum Zocken, Tipps, Coaching und alles rund ums Game.\n\n"
            "Kurz durchklicken lohnt sich – dann weißt du alles was du brauchst. Los geht's."
        ),
        "color": 0x5865F2,
    },
    # ── 1 ─────────────────────────────────────────────────────────────────
    {
        "title": "📋 Regeln & Verhalten",
        "description": (
            "Kurz & knapp – kein Roman:\n\n"
            "🚫 **Was gar nicht geht:**\n"
            "- Beleidigungen, Hassrede, Diskriminierung\n"
            "- Spam oder Werbung für andere Server\n"
            "- NSFW-Content irgendwo auf dem Server\n"
            "- Leute in Chill-Lanes wegen ihrem Skill anmachen\n\n"
            "✅ **Was wir erwarten:**\n"
            "- Respekt gegenüber allen – egal Rang, Erfahrung oder Spielstil\n"
            "- Im Voice **kommunizieren**: Lane-Gegner fehlen? Callt es. Ihr macht ne Box? Callt es. "
            "Ihr wechselt die Lane? Callt es. Niemand hat Bock auf 1v2 weil keiner redet.\n"
            "- Bei Problemen: kein Stress machen sondern ein Ticket öffnen -> "
            "#ticket-eroeffnen\n\n"
            "**Faustregel:** Behandel andere so wie du selbst behandelt werden willst."
        ),
        "color": 0xED4245,
    },
    # ── 2: Content Creator (Dynamic) ──────────────────────────────────────
    {
        "id": "content_creator",
        "title": "🎮 Streamst du Deadlock?",
        "description": (
            "Wir haben einen **exklusiven Streamer-Bereich** mit automatisierten Tools, "
            "die dir als Partner das Leben leichter machen.\n\n"
            "**1️⃣ Auto-Raid Manager**\n"
            "Schluss mit manuellem Raid-Suchen am Ende eines langen Streams. Der Bot übernimmt das automatisch:\n"
            "• Sobald dein Stream **offline** geht, prüft der Bot, **welche Partner aktuell live** sind und raidet einen davon\n"
            "• **Fallback:** Wenn **kein Partner live** ist, sucht der Bot automatisch nach **deutschen Deadlock-Streamern**\n\n"
            "• **Manuelle Raids gehen nach wie vor, und der Bot ist nur aktiv wenn du Deadlock Streamst**.\n\n"
            "**2️⃣ Chat Guard – Schutz vor Müll im Chat**\n"
            "Damit dein Chat sauber bleibt, ohne dass du ständig moderieren musst:\n"
            "• **Spam-Mod:** Filtert Viewer-Bots z.B. Scam-Links.\n\n"
            "**3️⃣ Analytics Dashboard**\n"
            "• **Retention-Analyse:** Wann droppen Zuschauer? (z. B. nach 5, 10 oder 20 Minuten)\n"
            "• **Unique Chatters:** Wie viele **verschiedene** Menschen interagieren wirklich?\n"
            "• **Kategorie-Vergleich (DE):** Analyse der deutschen Deadlock-Kategorie & Vergleich zwischen Streamern\n"
            "→ Ziel: Du erkennst Muster und weißt, was du optimieren kannst.\n"
            "→ **Sneak Peak gefällig?** Klick unten auf **„📊 Demo ansehen“**!\n\n"
            "**4️⃣ Discord – Live-Stream Auto-Post**\n"
            "• Sobald du **Deadlock** streamst, wird dein Stream automatisch im Discord gepostet (#🎥twitch)\n"
            "→ Ergebnis: Mehr Sichtbarkeit in der Community, ohne dass du selbst posten musst.\n\n"
            "Gib uns Feedback, wenn dir etwas auffällt oder du dir weitere Features wünschst.\n\n"
            "**Bereit, Partner zu werden?** Klicke unten auf **'Setup starten'**."
        ),
        "color": 0x9146FF,
    },
    # ── 3 ─────────────────────────────────────────────────────────────────
    {
        "title": "🎙️ Voice Lanes – was ist was?",
        "description": (
            "Es gibt verschiedene Lane-Typen und die unterscheiden sich wirklich:\n\n"
            "🏆 **Ranked / Competitive Lanes**\n"
            "Nur für Leute in deinem Rang-Bereich (±2 Ränge). Max. 6 Spieler pro Lane. "
            "Willst du den Skill-Diff noch enger halten? In "
            f"{_c(CH_TEMPVOICE)} kannst du den Mindestrang für deine Lane anpassen.\n"
            "→ Hier kann der Ton mal direkter sein – aber respektvoll bleibt ihr trotzdem.\n\n"
            "🎮 **Chill / Spaß Lanes**\n"
            "Kein Rang-Limit. Der Rang-Hinweis an der Lane ist nur eine grobe Orientierung – "
            "du kannst trotzdem joinen.\n"
            "→ **WICHTIG:** Hier nervt sich NIEMAND über Skill. Wer wegen schlechtem Gameplay "
            "flamet, kann das in Ranked Lanes machen. In Chill-Lanes ist das ein No-Go.\n\n"
            "🆕 **Neue Spieler Lane**\n"
            "Primär für Leute die noch neu im Game sind. Jeder kann joinen, aber kein Flame, "
            "kein 'warum weißt du das nicht' - neue Spieler lernen noch das Game, "
            "nicht auch noch den Server.\n\n"
            "🥊 **Street Brawl Lanes**\n"
            "Eigene Kategorie speziell für den Street Brawl Modus."
        ),
        "color": 0xFEE75C,
    },
    # ── 4 ─────────────────────────────────────────────────────────────────
    {
        "title": "🚧 Lane öffnen & verwalten",
        "description": (
            f"**Lane öffnen:** Geh in {_c(CH_TEMPVOICE)} – dort ist ein Dropdown-Menü. "
            "Lane-Typ auswählen, fertig. Du bist automatisch der **Owner** der Lane.\n\n"
            "**Als Owner hast du folgende Tools:**\n\n"
            "👢 **Kick** – Jemand ist AFK oder nervt und Reden hilft nicht? Raus damit.\n"
            "🚫 **Ban** – Willst du jemanden dauerhaft aus deiner Lane ausschließen? Ban setzen. "
            "Die Person kann nicht mehr beitreten solange du Owner bist.\n"
            "↩️ **Unban** – Ban wieder aufheben.\n"
            "👥 **Duo / Trio** – Nur zu zweit oder dritt? Aktivieren, dann kommt niemand anderes rein.\n"
            "🔄 **Normale Lane** – Duo/Trio aufheben, alles zurück auf Standard.\n"
            "👁️ **Lurker-Rolle** – Du schaust nur zu und spielst nicht mit? "
            "Lurker-Rolle annehmen = du schaffst einen extra Platz für jemanden der mitspielen will.\n\n"
            "**Owner-Wechsel:** Wenn der Owner die Lane verlässt kannst du die Lane übernehmen "
            "und bist dann der neue Owner."
        ),
        "color": 0x57F287,
    },
    # ── 5 ─────────────────────────────────────────────────────────────────
    {
        "title": "🎮 Mitspieler finden – so geht's richtig",
        "description": (
            "Das machen leider die meisten falsch, deswegen einmal klar erklärt:\n\n"
            "**Schritt 1 – Lanes checken (das Wichtigste!)**\n"
            "Schau im Seiten-Panel unter den Sprachkanälen was gerade offen ist. "
            "Gibt's eine Lane die halbwegs passt? → Einfach joinen und schauen. "
            "90% der Zeit passt es.\n\n"
            "**Schritt 2 – Erst wenn wirklich nix passt:**\n"
            f"Eigene Lane in {_c(CH_TEMPVOICE)} aufmachen und dann in "
            f"{_c(CH_LFG)} schreiben was du suchst. "
            "Der Bot schaut dann automatisch wer von den aktiven Spielern vom Rang her passt "
            "und zeigt dir das an – mit Status (Lobby / Match) und ob noch Platz ist.\n\n"
            "**Bitte nicht:** Direkt in spieler-suche schreiben ohne vorher zu schauen "
            "ob schon was offen ist. Die Lanes sind sichtbar – einfach kurz hinschauen.\n\n"
            "💡 **Tipp:** Wenn du die **LFG Ping Rolle** hast (Discord Onboarding bei Rollen-Auswahl), "
            "wirst du gepingt wenn jemand Mitspieler sucht."
        ),
        "color": 0x5865F2,
    },
    # ── 6 ─────────────────────────────────────────────────────────────────
    {
        "title": "🛠️ Was der Server sonst noch so hat",
        "description": (
            f"**📺 Clips & YouTube** – {_c(CH_CLIPS)}\n"
            "Deine besten Highlights dort einreichen. Wir bauen daraus YouTube Videos. "
            "Bester Clip der Woche wird von der Community gevotet – "
            "manchmal gibt's was zu gewinnen.\n\n"
            f"**#🛠️ich-brauch-einen-coach\n"
            "Du willst besser werden oder brauchst Hilfe? Dort anfragen, "
            "dann gehts in die #🏆Coaching Lane🏆 im Voice.\n\n"
            "**🧩 Custom Games** – #📍Sammelpunkt\n"
            "Wir machen regelmäßig Custom Games. Treffpunkt ist der **Sammelpunkt** Voice Channel, "
            "Koordination läuft über **#custom-games-chat**. Mit `/customgame` Befehlen kannst du Games erstellen. "
            "Wer die **Custom Games Ping Rolle** hat (Discord Onboarding → Rollen auswählen) "
            "wird gepingt wenn was läuft.\n\n"
            "**📝 Patchnotes** – #patchnotes\n"
            "Alle Patches auf Deutsch. Mit der **Patchnotes Ping Rolle** bekommst du sofort eine Benachrichtigung.\n\n"
            "**🎥 Streamer**\n"
            "Streamst du Deadlock? Mit `/streamer` beantragst du die Streamer-Partner-Rolle – läuft automatisch.\n\n"
            f"**🗝️ Kein Deadlock-Zugang?** – {_c(CH_BETA)}\n"
            "Dort einfach melden, wir helfen weiter.\n\n"
            "**Fast fertig – nur noch ein letzter Schritt! 🎮**"
        ),
        "color": 0x57F287,
    },
    # ── 7 ─────────────────────────────────────────────────────────────────
    {
        "title": "Voice-Ton",
        "description": (
            "Wähle optional, wie du meist im Voice unterwegs bist.\n\n"
            "**Banter-OK**\n"
            "Direktere Runden, in denen auch mal geflucht oder gestichelt wird.\n\n"
            "**Ragebaiter-Free**\n"
            "Runden ohne gezielte Provokationen, dafür mit strengerem Filter.\n\n"
            "Wenn du nichts auswählst, kannst du das später jederzeit per `/meine-tags` setzen."
        ),
        "color": 0x5865F2,
    },
    # ── 8 ─────────────────────────────────────────────────────────────────
    {
        "title": "Alter (optional)",
        "description": (
            "Wähle optional eine Altersgruppe für Voice-Filter.\n\n"
            "**25+**\n"
            "Für Runden, die bewusst mit 25+-Filter erstellt werden.\n\n"
            "**U25**\n"
            "Für Runden ohne 25+-Markierung.\n\n"
            "Wenn du diesen Schritt überspringst, kannst du den Tag später per `/meine-tags` setzen."
        ),
        "color": 0xFEE75C,
    },
    # ── 9 ─────────────────────────────────────────────────────────────────
    {
        "id": "steam_link",
        "title": "🔗 Account verknüpfen & Rang-System",
        "description": (
            "**Warum verknüpfen?**\n"
            "Mit verknüpftem Steam-Account wird dein Rang korrekt angezeigt und immer aktuell gehalten. "
            "Der Live-Status in den Voice Lanes funktioniert zuverlässig "
            "und die Spielersuche kann dich sauber einstufen.\n\n"
            "**So geht's:**\n"
            "Nutze einfach die **Buttons unten**, um deinen Steam-Account zu verknüpfen.\n"
            "Open Source: <https://github.com/NaniDerEchte2/Deadlock-Bots>\n"
            "Sobald der Bot dich verifiziert hat, ist dein Onboarding **abgeschlossen**.\n"
            "> Mehrere Accounts? Kein Problem – einfach mehrfach `/account_verknüpfen` ausführen.\n\n"
            "⚠️ **Wichtig:** Funktioniert nur korrekt wenn **alle im Call** ihren Account verknüpft haben – "
            "sonst sind die Angaben unvollständig."
        ),
        "color": 0x00AEEF,
    },
]


# Index des Account-Verknüpfen-Schritts
_TONE_STEP_INDEX = 7
_AGE_STEP_INDEX = 8
_ACCOUNT_STEP_INDEX = 9
_STREAMER_STEP_INDEX = 2


def _steam_link_urls_for(uid: int) -> dict[str, str]:
    if _steam_link_oauth is None:
        return {"discord_start": "", "steam_openid_start": ""}
    try:
        urls = _steam_link_oauth.start_urls_for(int(uid))
    except Exception:
        log.exception("Konnte Steam-Link-URLs für Onboarding nicht erzeugen (user_id=%s)", uid)
        return {"discord_start": "", "steam_openid_start": ""}
    if not isinstance(urls, dict):
        return {"discord_start": "", "steam_openid_start": ""}
    return {
        "discord_start": str(urls.get("discord_start") or "").strip(),
        "steam_openid_start": str(urls.get("steam_openid_start") or "").strip(),
    }


def _steam_friend_code_supported() -> bool:
    if _steam_link_oauth is None:
        return False
    return bool(
        getattr(_steam_link_oauth, "FRIEND_CODE_LINKING_ENABLED", False)
        and hasattr(_steam_link_oauth, "SteamFriendCodeModal")
    )


# ---------------------------------------------------------------------------
# Views
# ---------------------------------------------------------------------------


class NextStepView(discord.ui.View):
    """Zeigt einen 'Weiter ➜' Button für alle Schritte außer dem letzten."""

    def __init__(self, cog: StaticOnboarding, step_index: int, user_id: int):
        super().__init__(timeout=3600)  # 1 Stunde – kein Reboot-Persist nötig
        self.cog = cog
        self.step_index = step_index
        self.user_id = user_id

    @discord.ui.button(label="Weiter ➜", style=discord.ButtonStyle.primary)
    async def next_step(self, interaction: discord.Interaction, button: discord.ui.Button):
        if interaction.user.id != self.user_id:
            await interaction.response.send_message(
                "Dieses Onboarding gehört jemand anderem.", ephemeral=True
            )
            return

        next_index = self.step_index + 1

        # Streamer-Schritt überspringen, wenn der User kein Streamer ist
        if next_index == _STREAMER_STEP_INDEX:
            is_streamer = any(r.id == CONTENT_CREATOR_ROLE_ID for r in interaction.user.roles)
            if not is_streamer:
                next_index += 1
            else:
                # Spezial-View für Content Creator (1:1 Streamer Onboarding Buttons)
                view = OnboardingContentCreatorView(self.cog, next_index, self.user_id)
                embed = _build_embed(next_index, interaction.user)
                await interaction.response.send_message(embed=embed, view=view)
                self.stop()
                return

        if next_index == _ACCOUNT_STEP_INDEX:
            # Schritt: Account verknüpfen
            already_verified = any(r.id == VERIFIED_ROLE_ID for r in interaction.user.roles)

            # Immer OnboardingAccountLinkView nutzen (damit die Link-Buttons da sind)
            # Aber: "Weiter" Button nur zeigen wenn schon verifiziert
            view = OnboardingAccountLinkView(
                self.cog, next_index, self.user_id, show_next=already_verified
            )

            if not already_verified:
                await self.cog._register_pending_verify(self.user_id, interaction.channel.id)

            embed = _build_embed(next_index, interaction.user)
            await interaction.response.send_message(embed=embed, view=view)
            self.stop()
            return

        if next_index == _TONE_STEP_INDEX:
            embed = _build_embed(next_index, interaction.user)
            view = OnboardingToneStepView(self.cog, next_index, self.user_id)
            await interaction.response.send_message(embed=embed, view=view)
            self.stop()
            return

        if next_index == _AGE_STEP_INDEX:
            embed = _build_embed(next_index, interaction.user)
            view = OnboardingAgeStepView(self.cog, next_index, self.user_id)
            await interaction.response.send_message(embed=embed, view=view)
            self.stop()
            return

        embed = _build_embed(next_index, interaction.user)
        if next_index >= len(STEPS) - 1:
            view = DoneView(self.user_id)
        else:
            view = NextStepView(self.cog, next_index, self.user_id)

        await interaction.response.send_message(embed=embed, view=view)
        self.stop()


class OnboardingTagStepView(discord.ui.View):
    """Basisklasse fuer optionale Tag-Schritte im Onboarding."""

    def __init__(self, cog: StaticOnboarding, step_index: int, user_id: int):
        super().__init__(timeout=3600)
        self.cog = cog
        self.step_index = step_index
        self.user_id = user_id

    async def _store_tag_and_continue(
        self,
        interaction: discord.Interaction,
        *,
        key: str | None = None,
        value: str | None = None,
    ) -> None:
        if interaction.user.id != self.user_id:
            await interaction.response.send_message(
                "Dieses Onboarding gehoert jemand anderem.", ephemeral=True
            )
            return

        if key is not None and value is not None:
            tag_service = self.cog.bot.get_cog("TagService")
            if tag_service is not None:
                await tag_service.set_user_tag(self.user_id, key, value)
            else:
                log.warning(
                    "TagService nicht geladen, Onboarding-Tag wurde nicht gespeichert (user_id=%s, key=%s)",
                    self.user_id,
                    key,
                )

        next_index = self.step_index + 1

        if next_index == _AGE_STEP_INDEX:
            embed = _build_embed(next_index, interaction.user)
            view = OnboardingAgeStepView(self.cog, next_index, self.user_id)
            await interaction.response.send_message(embed=embed, view=view)
            self.stop()
            return

        if next_index == _ACCOUNT_STEP_INDEX:
            already_verified = any(r.id == VERIFIED_ROLE_ID for r in interaction.user.roles)
            view = OnboardingAccountLinkView(
                self.cog, next_index, self.user_id, show_next=already_verified
            )

            if not already_verified:
                await self.cog._register_pending_verify(self.user_id, interaction.channel.id)

            embed = _build_embed(next_index, interaction.user)
            await interaction.response.send_message(embed=embed, view=view)
            self.stop()
            return

        embed = _build_embed(next_index, interaction.user)
        view = DoneView(self.user_id) if next_index >= len(STEPS) - 1 else NextStepView(
            self.cog, next_index, self.user_id
        )
        await interaction.response.send_message(embed=embed, view=view)
        self.stop()


class OnboardingToneStepView(OnboardingTagStepView):
    """Optionaler Onboarding-Schritt fuer den bevorzugten Voice-Ton."""

    @discord.ui.button(label="Banter-OK", style=discord.ButtonStyle.primary, row=0)
    async def banter_ok(self, interaction: discord.Interaction, button: discord.ui.Button):
        await self._store_tag_and_continue(interaction, key="tone", value="banter_ok")

    @discord.ui.button(label="Ragebaiter-Free", style=discord.ButtonStyle.secondary, row=0)
    async def ragebaiter_free(
        self, interaction: discord.Interaction, button: discord.ui.Button
    ):
        await self._store_tag_and_continue(interaction, key="tone", value="ragebaiter_free")

    @discord.ui.button(label="Überspringen", style=discord.ButtonStyle.secondary, row=1)
    async def skip(self, interaction: discord.Interaction, button: discord.ui.Button):
        await self._store_tag_and_continue(interaction)


class OnboardingAgeStepView(OnboardingTagStepView):
    """Optionaler Onboarding-Schritt fuer die Altersgruppe."""

    @discord.ui.button(label="25+", style=discord.ButtonStyle.primary, row=0)
    async def age_25_plus(self, interaction: discord.Interaction, button: discord.ui.Button):
        await self._store_tag_and_continue(interaction, key="age", value="25+")

    @discord.ui.button(label="U25", style=discord.ButtonStyle.secondary, row=0)
    async def age_u25(self, interaction: discord.Interaction, button: discord.ui.Button):
        await self._store_tag_and_continue(interaction, key="age", value="u25")

    @discord.ui.button(label="Überspringen", style=discord.ButtonStyle.secondary, row=1)
    async def skip(self, interaction: discord.Interaction, button: discord.ui.Button):
        await self._store_tag_and_continue(interaction)


class OnboardingContentCreatorView(discord.ui.View):
    """
    Spezialisierte View für den Content-Creator-Schritt:
    Enthält Demo-Link, Setup-Start-Button und Weiter-Button.
    """

    def __init__(self, cog: StaticOnboarding, step_index: int, user_id: int):
        super().__init__(timeout=3600)
        self.cog = cog
        self.step_index = step_index
        self.user_id = user_id

        self.add_item(
            discord.ui.Button(
                label="📊 Demo ansehen",
                style=discord.ButtonStyle.link,
                url="https://deutsche-deadlock-community.de/demo/twitch/demo/",
                row=0,
            )
        )

    @discord.ui.button(
        label="Ja, Partner werden / Setup starten 🎥",
        style=discord.ButtonStyle.success,
        row=0,
    )
    async def start_setup(self, interaction: discord.Interaction, button: discord.ui.Button):
        if interaction.user.id != self.user_id:
            await interaction.response.send_message(
                "Das ist nicht dein Onboarding.", ephemeral=True
            )
            return

        # Trigger den Streamer-Setup-Flow (DMs)
        try:
            from cogs.welcome_dm.step_streamer import StreamerIntroView

            dm = await interaction.user.create_dm()
            await dm.send(
                embed=StreamerIntroView.build_embed(interaction.user),
                view=StreamerIntroView(),
            )
            await interaction.response.send_message(
                "✅ **Setup gestartet!** Ich habe dir alle Details in deine DMs geschickt.\n"
                "Du kannst hier im Onboarding währenddessen einfach auf 'Weiter' klicken.",
                ephemeral=True,
            )
        except discord.Forbidden:
            await interaction.response.send_message(
                "⚠️ Ich konnte dir keine DM senden. Bitte aktiviere DMs für diesen Server.",
                ephemeral=True,
            )
        except Exception:
            log.exception("Konnte Streamer-Setup aus Onboarding nicht starten")
            await interaction.response.send_message(
                "⚠️ Unerwarteter Fehler beim Starten des Setups.", ephemeral=True
            )

    @discord.ui.button(label="Weiter ➜", style=discord.ButtonStyle.primary, row=1)
    async def next_step(self, interaction: discord.Interaction, button: discord.ui.Button):
        if interaction.user.id != self.user_id:
            await interaction.response.send_message(
                "Das ist nicht dein Onboarding.", ephemeral=True
            )
            return

        next_index = self.step_index + 1
        embed = _build_embed(next_index, interaction.user)

        # Nach CC kommt immer der Voice-Lane Schritt (Index 3)
        view = NextStepView(self.cog, next_index, self.user_id)
        await interaction.response.send_message(embed=embed, view=view)
        self.stop()


class OnboardingAccountLinkView(discord.ui.View):
    """
    Spezialisierte View für den Account-Verknüpfen-Schritt:
    Enthält die Steam-Link-Buttons (URL-Buttons).
    'Weiter' Button wird nur gezeigt, wenn der User bereits verifiziert ist.
    """

    def __init__(
        self, cog: StaticOnboarding, step_index: int, user_id: int, show_next: bool = False
    ):
        super().__init__(timeout=3600)
        self.cog = cog
        self.step_index = step_index
        self.user_id = user_id
        self._steam_urls = _steam_link_urls_for(int(user_id))

        steam_url = self._steam_urls.get("steam_openid_start") or ""

        if steam_url:
            self.add_item(
                discord.ui.Button(
                    label="Via Steam verknüpfen",
                    style=discord.ButtonStyle.link,
                    url=steam_url,
                    emoji="🎮",
                    row=0,
                )
            )
        else:
            self.add_item(
                discord.ui.Button(
                    label="Steam-Link gerade nicht verfügbar",
                    style=discord.ButtonStyle.secondary,
                    disabled=True,
                    row=0,
                )
            )

        if _steam_friend_code_supported():
            btn = discord.ui.Button(
                label="Freundescode",
                style=discord.ButtonStyle.secondary,
                emoji="👥",
                row=0,
            )
            btn.callback = self.friend_code
            self.add_item(btn)

        if show_next:
            btn = discord.ui.Button(label="Weiter ➜", style=discord.ButtonStyle.primary, row=1)
            btn.callback = self.next_step
            self.add_item(btn)
        else:
            # Fallback: Manueller Refresh-Button falls automatische Erkennung klemmt
            btn = discord.ui.Button(
                label="Status prüfen 🔄", style=discord.ButtonStyle.secondary, row=1
            )
            btn.callback = self.refresh_status
            self.add_item(btn)

    async def friend_code(self, interaction: discord.Interaction):
        if interaction.user.id != self.user_id:
            await interaction.response.send_message(
                "Das ist nicht dein Onboarding.", ephemeral=True
            )
            return

        oauth_mod: Any = _steam_link_oauth
        if oauth_mod is None or not _steam_friend_code_supported():
            await interaction.response.send_message(
                "Der Freundescode-Flow ist aktuell nicht verfügbar. Nutze bitte den Steam-Login-Button.",
                ephemeral=True,
            )
            return

        link_cog = interaction.client.get_cog("SteamLink")
        if link_cog is None:
            await interaction.response.send_message(
                "⚠️ Steam-Link-Cog nicht geladen.",
                ephemeral=True,
            )
            return

        modal_cls = getattr(oauth_mod, "SteamFriendCodeModal", None)
        if modal_cls is None:
            await interaction.response.send_message(
                "Der Freundescode-Flow ist aktuell nicht verfügbar. Nutze bitte den Steam-Login-Button.",
                ephemeral=True,
            )
            return

        await interaction.response.send_modal(
            modal_cls(user_id=int(interaction.user.id), link_cog=link_cog)
        )

    async def refresh_status(self, interaction: discord.Interaction):
        if interaction.user.id != self.user_id:
            await interaction.response.send_message(
                "Das ist nicht dein Onboarding.", ephemeral=True
            )
            return

        already_verified = any(r.id == VERIFIED_ROLE_ID for r in interaction.user.roles)
        if already_verified:
            await self.next_step(interaction)
        else:
            await interaction.response.send_message(
                "Du hast die **Verified**-Rolle noch nicht. Bitte stelle sicher, dass du deinen Account verknüpft hast "
                "und dem Steam-Bot (Freundescode 820142646) eine Freundschaftsanfrage geschickt und angenommen hast. "
                "(Es kann ein paar Minuten dauern)",
                ephemeral=True,
            )

    async def next_step(self, interaction: discord.Interaction):
        if interaction.user.id != self.user_id:
            await interaction.response.send_message(
                "Dieses Onboarding gehört jemand anderem.", ephemeral=True
            )
            return

        next_index = self.step_index + 1

        # Falls es nach Steam Link noch was gäbe (aktuell letzter Schritt)
        if next_index >= len(STEPS):
            view = DoneView(self.user_id)
            await interaction.response.send_message(
                "Nice, jetzt weißt du alles! Falls doch mal Fragen sind: "
                "einfach ein Ticket aufmachen oder einen Mod fragen. Have fun! 🎮",
                ephemeral=True,
                view=view,
            )
            self.stop()
            return

        embed = _build_embed(next_index, interaction.user)
        view = DoneView(self.user_id)  # Letzter Schritt nach Steam Link

        await interaction.response.send_message(embed=embed, view=view)
        self.stop()


class DoneView(discord.ui.View):
    """Letzter Schritt: Abschluss-Button."""

    def __init__(self, user_id: int):
        super().__init__(timeout=3600)
        self.user_id = user_id

    @discord.ui.button(label="Alles klar, viel Spaß! 🎮", style=discord.ButtonStyle.success)
    async def done(self, interaction: discord.Interaction, button: discord.ui.Button):
        if interaction.user.id != self.user_id:
            await interaction.response.send_message(
                "Dieses Onboarding gehört jemand anderem.", ephemeral=True
            )
            return

        await interaction.response.send_message(
            "Nice, jetzt weißt du alles! Falls doch mal Fragen sind: "
            "einfach ein Ticket aufmachen oder einen Mod fragen. Have fun! 🎮",
            ephemeral=True,
        )
        self.stop()


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _build_embed(step_index: int, user: discord.Member | None = None) -> discord.Embed:
    step = STEPS[step_index]
    embed = discord.Embed(
        title=step["title"],
        description=step["description"],
        color=step["color"],
    )

    # Dynamische Footer-Berechnung
    is_streamer = False
    if user:
        is_streamer = any(r.id == CONTENT_CREATOR_ROLE_ID for r in user.roles)

    total_steps = 10 if is_streamer else 9
    display_step = step_index + 1
    if not is_streamer and step_index > _STREAMER_STEP_INDEX:
        display_step -= 1

    embed.set_footer(text=f"Deutsche Deadlock Community · Schritt {display_step} / {total_steps}")
    return embed


# ---------------------------------------------------------------------------
# Cog
# ---------------------------------------------------------------------------


class StaticOnboarding(commands.Cog):
    """Statisches Multi-Step Onboarding – 7-8 Schritte, kein AI."""

    def __init__(self, bot: commands.Bot):
        self.bot = bot
        # In-Memory Cache (wird bei Start aus DB befüllt)
        self._pending_verify: dict[int, int] = {}

    async def cog_load(self):
        await self._db_ensure_schema()
        await self._db_load_pending()
        log.info(
            "StaticOnboarding geladen (%d Schritte, %d wartende Verifizierungen).",
            len(STEPS),
            len(self._pending_verify),
        )

    async def _db_ensure_schema(self):
        from service import db

        await db.execute_async("""
            CREATE TABLE IF NOT EXISTS onboarding_pending_verify (
                user_id INTEGER PRIMARY KEY,
                channel_id INTEGER NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        """)

    async def _db_load_pending(self):
        from service import db

        rows = await db.query_all_async("SELECT user_id, channel_id FROM onboarding_pending_verify")
        self._pending_verify = {r["user_id"]: r["channel_id"] for r in rows}

    async def _register_pending_verify(self, user_id: int, channel_id: int):
        from service import db

        self._pending_verify[user_id] = channel_id
        await db.execute_async(
            "INSERT INTO onboarding_pending_verify(user_id, channel_id) VALUES(?, ?) "
            "ON CONFLICT(user_id) DO UPDATE SET channel_id=excluded.channel_id, updated_at=CURRENT_TIMESTAMP",
            (user_id, channel_id),
        )

    async def _pop_pending_verify(self, user_id: int) -> int | None:
        from service import db

        channel_id = self._pending_verify.pop(user_id, None)
        if channel_id:
            await db.execute_async(
                "DELETE FROM onboarding_pending_verify WHERE user_id=?", (user_id,)
            )
        return channel_id

    @commands.Cog.listener()
    async def on_member_update(self, before: discord.Member, after: discord.Member):
        """Sendet Abschluss-Nachricht automatisch sobald die Verified-Rolle vergeben wird."""
        if after.guild.id != GUILD_ID:
            return
        had_role = any(r.id == VERIFIED_ROLE_ID for r in before.roles)
        has_role = any(r.id == VERIFIED_ROLE_ID for r in after.roles)
        if not had_role and has_role:
            channel_id = await self._pop_pending_verify(after.id)
            if channel_id:
                channel = self.bot.get_channel(channel_id)
                if not channel:
                    try:
                        channel = await self.bot.fetch_channel(channel_id)
                    except Exception:
                        log.warning(
                            "Konnte Onboarding-Channel %s nicht finden für User %s",
                            channel_id,
                            after.id,
                        )
                        return

                if channel:
                    try:
                        await channel.send(
                            content=f"<@{after.id}> ✅ **Verifizierung erfolgreich!**\n\n"
                            "Nice, jetzt weißt du alles! Falls doch mal Fragen sind: "
                            "einfach ein Ticket aufmachen oder einen Mod fragen. Have fun! 🎮",
                            view=DoneView(after.id),
                        )
                    except Exception:
                        log.exception(
                            "Konnte Abschluss-Nachricht nicht senden für User %s in Channel %s",
                            after.id,
                            channel_id,
                        )

    # Öffentliche API – kompatibel mit rules_channel.py
    async def start_in_channel(
        self, channel: discord.abc.Messageable, member: discord.Member
    ) -> bool:
        """Postet Schritt 0 in den Thread/Channel und startet den Flow."""
        try:
            embed = _build_embed(0, member)
            view = NextStepView(self, step_index=0, user_id=member.id)
            await channel.send(embed=embed, view=view)
            return True
        except Exception:
            log.exception("StaticOnboarding konnte nicht gestartet werden")
            return False


async def setup(bot: commands.Bot):
    await bot.add_cog(StaticOnboarding(bot))
