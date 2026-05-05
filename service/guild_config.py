"""
Zentrale Guild-spezifische Konfiguration.
Verschiebt hardcoded IDs aus verschiedenen Cogs in eine Config.

Neue Lane-Struktur (kein Ranked/Grind Split mehr):
  Chill Lanes   → Kategorie 1289721245281292290 | Staging 1501089974093873232
  Comp/Ranked   → Kategorie 1412804540994162789 | Staging 1412804671432818890
  Street Brawl  → Kategorie 1357422957017698478 | Staging 1357422958544420944
"""

from dataclasses import dataclass


@dataclass
class GuildIDs:
    """Hardcoded Guild/Channel/Role IDs - zentral verwaltet."""

    # TempVoice Staging Channels
    TEMPVOICE_STAGING_CASUAL: int = 1501089974093873232
    TEMPVOICE_STAGING_STREET_BRAWL: int = 1357422958544420944
    TEMPVOICE_STAGING_COMP: int = 1412804671432818890
    TEMPVOICE_STAGING_RANKED: int = TEMPVOICE_STAGING_COMP  # Alias für Altcode
    TEMPVOICE_PERMANENT_CASUAL_CHANNEL: int = 1493690350580138114

    # TempVoice Kategorien
    TEMPVOICE_CATEGORY_CHILL: int = 1289721245281292290
    TEMPVOICE_CATEGORY_COMP: int = 1412804540994162789  # Comp/Ranked (war: Grind)
    TEMPVOICE_CATEGORY_STREET_BRAWL: int = 1357422957017698478

    # Legacy-Aliases (Altcode-Kompatibilität)
    TEMPVOICE_CATEGORY_GRIND: int = TEMPVOICE_CATEGORY_COMP
    TEMPVOICE_CATEGORY_NORMAL: int = TEMPVOICE_CATEGORY_CHILL
    TEMPVOICE_CATEGORY_RANKED: int = TEMPVOICE_CATEGORY_COMP

    TEMPVOICE_INTERFACE_CHANNEL: int = 1371927143537315890
    ENGLISH_ONLY_ROLE: int = 1309741866098491479

    # Rank Voice Manager
    RANK_CATEGORY_CASUAL_1: int = 1290343267974406234
    RANK_CATEGORY_CASUAL_2: int = 1332065827867230339
    RANK_CATEGORY_RANKED_1: int = 1290343215927070732
    RANK_CATEGORY_RANKED_2: int = 1332065944548712560

    # Deadlock Voice Status
    VOICE_STATUS_CATEGORY_COMP: int = 1412804540994162789
    VOICE_STATUS_CATEGORY_CHILL: int = 1289721245281292290
    VOICE_STATUS_CATEGORY_STREET_BRAWL: int = 1357422957017698478

    # Legacy-Aliases Voice Status
    VOICE_STATUS_CATEGORY_GRIND: int = VOICE_STATUS_CATEGORY_COMP
    VOICE_STATUS_CATEGORY_CASUAL: int = VOICE_STATUS_CATEGORY_CHILL
    VOICE_STATUS_CATEGORY_RANKED: int = VOICE_STATUS_CATEGORY_COMP

    # Steam Verified Role
    STEAM_VERIFIED_GUILD_ID: int = 1075099396802904185
    STEAM_VERIFIED_ROLE_ID: int = 1313559094994391063
    STEAM_VERIFIED_ANNOUNCE_CHANNEL_ID: int = 1313587932169859083

    @property
    def tempvoice_staging_channels(self) -> set[int]:
        """Alle TempVoice Staging Channel IDs."""
        return {
            self.TEMPVOICE_STAGING_CASUAL,
            self.TEMPVOICE_STAGING_STREET_BRAWL,
            self.TEMPVOICE_STAGING_COMP,
        }

    @property
    def tempvoice_minrank_categories(self) -> set[int]:
        """Kategorie-IDs wo MinRank aktiv ist (nur Comp/Ranked)."""
        return {self.TEMPVOICE_CATEGORY_COMP}

    @property
    def rank_voice_categories(self) -> set[int]:
        """Alle Rank Voice Manager Category IDs."""
        return {
            self.RANK_CATEGORY_CASUAL_1,
            self.RANK_CATEGORY_CASUAL_2,
            self.RANK_CATEGORY_RANKED_1,
            self.RANK_CATEGORY_RANKED_2,
            self.TEMPVOICE_CATEGORY_COMP,
        }

    @property
    def voice_status_categories(self) -> set[int]:
        """Alle Voice Status Category IDs."""
        return {
            self.VOICE_STATUS_CATEGORY_COMP,
            self.VOICE_STATUS_CATEGORY_CHILL,
            self.VOICE_STATUS_CATEGORY_STREET_BRAWL,
        }


# Singleton instance
_guild_ids: GuildIDs | None = None


def get_guild_config() -> GuildIDs:
    """
    Zugriff auf Guild Config (Singleton).

    Kann später auf ENV-basierte Overrides erweitert werden:
    ```python
    TEMPVOICE_STAGING_CASUAL = int(os.getenv(
        "TEMPVOICE_STAGING_CASUAL",
        str(defaults.TEMPVOICE_STAGING_CASUAL)
    ))
    ```
    """
    global _guild_ids
    if _guild_ids is None:
        _guild_ids = GuildIDs()

    return _guild_ids
