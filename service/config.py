import logging
import os
from pathlib import Path

from pydantic import Field, SecretStr
from pydantic_settings import BaseSettings, SettingsConfigDict

log = logging.getLogger(__name__)


def _keyring_enabled() -> bool:
    override = (os.getenv("DEADLOCK_ENABLE_KEYRING") or "").strip().lower()
    if override in {"1", "true", "yes", "on"}:
        return True
    if override in {"0", "false", "no", "off"}:
        return False
    return os.name == "nt"


def _load_vault_secrets():
    """Injiziert Secrets aus dem Windows Tresor in os.environ."""
    if not _keyring_enabled():
        log.debug("Keyring/Tresor-Check deaktiviert.")
        return

    try:
        import keyring

        service_name = "DeadlockBot"
        keys = [
            "DISCORD_TOKEN",
            "OWNER_ID",
            "COMMAND_PREFIX",
            "STEAM_API_KEY",
            "TWITCH_CLIENT_ID",
            "TWITCH_CLIENT_SECRET",
            "OPENAI_API_KEY",
            "GEMINI_API_KEY",
            "PUBLIC_BASE_URL",
            "TIKTOK_CLIENT_KEY",
            "TIKTOK_CLIENT_SECRET",
            "YOUTUBE_CLIENT_ID",
            "YOUTUBE_CLIENT_SECRET",
            "INSTAGRAM_CLIENT_ID",
            "INSTAGRAM_CLIENT_SECRET",
        ]
        count = 0
        for key in keys:
            # IMMER aus Tresor laden und überschreiben, das ist die sicherste Quelle
            val = keyring.get_password(service_name, key)
            if val:
                os.environ[key] = val
                count += 1
        # print(f"DEBUG: Config injizierte {count} Secrets aus Tresor.")
    except Exception as e:
        log.warning("Fehler beim Laden aus Tresor: %s", e)


# Vor der Klassen-Definition aufrufen!
_load_vault_secrets()


class Settings(BaseSettings):
    # --- Bot Core ---
    discord_token: SecretStr = Field(..., alias="DISCORD_TOKEN")
    owner_id: int = Field(0, alias="OWNER_ID")
    command_prefix: str = Field("!", alias="COMMAND_PREFIX")
    log_level: str = Field("INFO", alias="LOG_LEVEL")
    guild_id: int = Field(1289721245281292288, alias="GUILD_ID")

    # --- Database ---
    deadlock_db_path: Path | None = Field(None, alias="DEADLOCK_DB_PATH")
    deadlock_db_dir: Path | None = Field(None, alias="DEADLOCK_DB_DIR")
    deadlock_db_busy_timeout_ms: int = Field(15000, alias="DEADLOCK_DB_BUSY_TIMEOUT_MS")

    # --- Steam / Deadlock Integration ---
    steam_api_key: SecretStr | None = Field(None, alias="STEAM_API_KEY")
    steam_web_api_key: SecretStr | None = Field(None, alias="STEAM_WEB_API_KEY")
    # Public OAuth endpoints are exposed behind the shared /link reverse proxy path.
    public_base_url: str = Field(
        "https://deutsche-deadlock-community.de/link", alias="PUBLIC_BASE_URL"
    )
    steam_poll_min_interval_sec: int = Field(86400, alias="STEAM_POLL_MIN_INTERVAL_SEC")
    steam_unfollow_miss_threshold: int = Field(2, alias="STEAM_UNFOLLOW_MISS_THRESHOLD")
    steam_poll_batch_size: int = Field(25, alias="STEAM_POLL_BATCH_SIZE")

    # --- Roles & Channels ---
    verified_role_id: int = Field(1419608095533043774, alias="VERIFIED_ROLE_ID")
    verified_log_channel_id: int = Field(1374364800817303632, alias="VERIFIED_LOG_CHANNEL_ID")
    streamer_role_id: int = Field(1313624729466441769, alias="STREAMER_ROLE_ID")
    content_creator_role_id: int = Field(1466630749255106590, alias="CONTENT_CREATOR_ROLE_ID")
    streamer_notify_channel_id: int = Field(1374364800817303632, alias="STREAMER_NOTIFY_CHANNEL_ID")
    coach_role_id: int = Field(1494372744286965941, alias="COACH_ROLE_ID")
    coaching_active_role_id: int = Field(1371929762913587292, alias="COACHING_ACTIVE_ROLE_ID")
    coaching_reward_role_id: int = Field(1500793970714873927, alias="COACHING_REWARD_ROLE_ID")
    coaching_request_channel_id: int = Field(
        1461682293105229979, alias="COACHING_REQUEST_CHANNEL_ID"
    )
    coaching_feedback_channel_id: int = Field(1494756126644895885, alias="COACHING_FEEDBACK_CHANNEL_ID")
    coaching_panel_channel_id: int = Field(1494373349944459355, alias="COACHING_PANEL_CHANNEL_ID")
    coaching_voice_category_id: int = Field(1459526231686119600, alias="COACHING_VOICE_CATEGORY_ID")
    coaching_role_expiry_hours: int = Field(48, alias="COACHING_ROLE_EXPIRY_HOURS")
    coaching_reminders_enabled: bool = Field(False, alias="COACHING_REMINDERS_ENABLED")

    # --- Steam Link UI & OAuth ---
    steam_return_path: str = Field("/callback/steam", alias="STEAM_RETURN_PATH")
    http_host: str = Field("127.0.0.1", alias="HTTP_HOST")
    http_port: int = Field(8888, alias="STEAM_OAUTH_PORT")
    link_cover_image: str = Field("", alias="LINK_COVER_IMAGE")
    link_cover_label: str = Field("link.deutsche-deadlock-community.de", alias="LINK_COVER_LABEL")
    link_button_label: str = Field("Via Discord verknüpfen", alias="LINK_BUTTON_LABEL")
    steam_button_label: str = Field("Direkt bei Steam anmelden", alias="STEAM_BUTTON_LABEL")
    steam_login_launch_ttl_sec: int = Field(900, alias="STEAM_LOGIN_LAUNCH_TTL_SEC")
    discord_oauth_redirect: str | None = Field(None, alias="DISCORD_OAUTH_REDIRECT")

    # --- TempVoice IDs ---
    # TODO: Move hardcoded sets from cogs/tempvoice/core.py here eventually
    # For now, we keep them there to avoid massive refactoring at once.

    # --- Deadlock Voice Status ---
    match_minute_offset: int = Field(3, alias="DEADLOCK_MATCH_MINUTE_OFFSET")
    rank_vs_rename_cooldown_seconds: int = Field(360, alias="RANK_VS_RENAME_COOLDOWN_SECONDS")

    # --- Feature Flags & Toggles ---
    master_dashboard_enabled: bool = Field(True, alias="MASTER_DASHBOARD_ENABLED")

    # --- External Worker Config ---
    # rename_worker_url: Optional[str] = Field(None, alias="RENAME_WORKER_URL") # Not used with DB communication
    use_db_rename_worker: bool = Field(False, alias="USE_DB_RENAME_WORKER")
    rename_worker_bot_token: SecretStr | None = Field(None, alias="RENAME_WORKER_BOT_TOKEN")

    model_config = SettingsConfigDict(
        env_file=".env",
        env_file_encoding="utf-8",
        extra="ignore",  # Ignore extra env vars
    )


# Singleton instance
try:
    settings = Settings()
except Exception as e:
    # Fallback for first-time setup or missing env
    log.warning("Config loading warning: %s", e)

    # Create a dummy instance to prevent import errors during setup
    # Real validation happens when the bot starts properly
    class DummySettings(Settings):
        discord_token: SecretStr = SecretStr("dummy")

    settings = DummySettings()

# Touch the singleton once locally so static analysis knows it is intentional.
log.debug("Config loaded; dashboard=%s", getattr(settings, "master_dashboard_enabled", None))


def get_settings() -> Settings:
    """Return the shared settings instance used across the bot."""
    return settings


__all__ = ["Settings", "settings", "get_settings"]
