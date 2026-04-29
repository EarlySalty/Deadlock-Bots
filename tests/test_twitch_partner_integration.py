from __future__ import annotations

import unittest
from pathlib import Path
from unittest import mock
from types import SimpleNamespace

from cogs.welcome_dm import twitch_partner_integration as integration


class _FakeResolver:
    state = None
    calls: list[dict[str, object]] = []

    def __init__(self, *, auth_manager=None, token_error_handler=None) -> None:
        self.auth_manager = auth_manager
        self.token_error_handler = token_error_handler

    def resolve_auth_state(self, discord_user_id: str):
        self.__class__.calls.append(
            {"method": "resolve_auth_state", "discord_user_id": discord_user_id}
        )
        return self.__class__.state

    def resolve_block_state(self, *, discord_user_id=None, twitch_login=None):
        self.__class__.calls.append(
            {
                "method": "resolve_block_state",
                "discord_user_id": discord_user_id,
                "twitch_login": twitch_login,
            }
        )
        return self.__class__.state


class TwitchPartnerIntegrationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.prev_modules = integration._EXTERNAL_MODULES
        self.prev_auth_manager = integration._AUTH_MANAGER
        _FakeResolver.calls = []
        integration._EXTERNAL_MODULES = integration._ExternalModules(
            repo_path=Path("."),
            raid_auth_manager_cls=object,
            raid_integration_state_resolver_cls=_FakeResolver,
            default_redirect_uri="https://example.invalid/callback",
        )
        integration._AUTH_MANAGER = SimpleNamespace(
            token_error_handler=SimpleNamespace(name="handler")
        )

    def tearDown(self) -> None:
        integration._EXTERNAL_MODULES = self.prev_modules
        integration._AUTH_MANAGER = self.prev_auth_manager

    def test_get_auth_state_uses_integration_resolver(self) -> None:
        _FakeResolver.state = SimpleNamespace(
            twitch_login="MasterIOFPS",
            twitch_user_id="153828567",
            authorized=True,
        )

        state = integration.get_auth_state(265152027863023617)

        self.assertEqual(state.twitch_login, "masteriofps")
        self.assertEqual(state.twitch_user_id, "153828567")
        self.assertTrue(state.authorized)
        self.assertEqual(
            _FakeResolver.calls,
            [{"method": "resolve_auth_state", "discord_user_id": "265152027863023617"}],
        )

    def test_generate_discord_auth_url_binds_discord_user_id_in_state(self) -> None:
        seen_calls: list[dict[str, object]] = []

        class _FakeAuthManager:
            token_error_handler = SimpleNamespace(name="handler")

            def generate_discord_button_url(self, login: str, **kwargs):
                seen_calls.append({"login": login, **kwargs})
                return "https://auth.example/raid"

        integration._AUTH_MANAGER = _FakeAuthManager()

        auth_url = integration.generate_discord_auth_url(265152027863023617)

        self.assertEqual(auth_url, "https://auth.example/raid")
        self.assertEqual(
            seen_calls,
            [
                {
                    "login": "discord:265152027863023617",
                    "discord_user_id": 265152027863023617,
                }
            ],
        )

    def test_check_onboarding_blocklist_uses_partner_state(self) -> None:
        _FakeResolver.state = SimpleNamespace(
            twitch_login="MasterIOFPS",
            twitch_user_id="153828567",
            partner_opt_out=False,
            token_blacklisted=False,
            raid_blacklisted=True,
        )

        blocked, reason = integration.check_onboarding_blocklist(discord_user_id=265152027863023617)

        self.assertTrue(blocked)
        self.assertEqual(reason, "twitch_raid_blacklist fuer masteriofps")
        self.assertEqual(
            _FakeResolver.calls,
            [
                {
                    "method": "resolve_block_state",
                    "discord_user_id": "265152027863023617",
                    "twitch_login": None,
                }
            ],
        )

    def test_generate_discord_auth_url_prefers_internal_api(self) -> None:
        calls: list[tuple[str, dict[str, object | None]]] = []

        def _fake_request(path: str, query: dict[str, object | None]):
            calls.append((path, query))
            return {"auth_url": "https://auth.example/internal"}

        with mock.patch.dict(
            integration.os.environ,
            {"TWITCH_INTERNAL_API_TOKEN": "secret-token"},
            clear=False,
        ), mock.patch.object(integration, "_request_internal_api_json", _fake_request):
            auth_url = integration.generate_discord_auth_url(265152027863023617)

        self.assertEqual(auth_url, "https://auth.example/internal")
        self.assertEqual(
            calls,
            [
                (
                    "/internal/twitch/v1/raid/auth-url",
                    {
                        "login": "discord:265152027863023617",
                        "discord_user_id": 265152027863023617,
                    },
                )
            ],
        )

    def test_internal_api_config_accepts_main_bot_token_fallback(self) -> None:
        with mock.patch.dict(
            integration.os.environ,
            {
                "TWITCH_INTERNAL_API_TOKEN": "",
                "MASTER_BROKER_TOKEN": "",
                "MAIN_BOT_INTERNAL_TOKEN": "main-bot-token",
                "TWITCH_INTERNAL_API_BASE_URL": "http://127.0.0.1:8776",
            },
            clear=False,
        ):
            config = integration._internal_api_config()

        self.assertIsNotNone(config)
        assert config is not None
        self.assertEqual(config[0], "http://127.0.0.1:8776")
        self.assertEqual(config[1], "main-bot-token")

    def test_get_auth_state_prefers_internal_api(self) -> None:
        def _fake_request(path: str, query: dict[str, object | None]):
            self.assertEqual(path, "/internal/twitch/v1/raid/auth-state")
            self.assertEqual(query, {"discord_user_id": 265152027863023617})
            return {
                "twitch_login": "MasterIOFPS",
                "twitch_user_id": "153828567",
                "authorized": True,
            }

        with mock.patch.dict(
            integration.os.environ,
            {"TWITCH_INTERNAL_API_TOKEN": "secret-token"},
            clear=False,
        ), mock.patch.object(integration, "_request_internal_api_json", _fake_request):
            state = integration.get_auth_state(265152027863023617)

        self.assertEqual(state.twitch_login, "masteriofps")
        self.assertEqual(state.twitch_user_id, "153828567")
        self.assertTrue(state.authorized)

    def test_check_onboarding_blocklist_prefers_internal_api(self) -> None:
        def _fake_request(path: str, query: dict[str, object | None]):
            self.assertEqual(path, "/internal/twitch/v1/raid/block-state")
            self.assertEqual(
                query,
                {
                    "discord_user_id": 265152027863023617,
                    "twitch_login": "masteriofps",
                },
            )
            return {
                "twitch_login": "MasterIOFPS",
                "twitch_user_id": "153828567",
                "partner_opt_out": False,
                "token_blacklisted": False,
                "raid_blacklisted": True,
            }

        with mock.patch.dict(
            integration.os.environ,
            {"TWITCH_INTERNAL_API_TOKEN": "secret-token"},
            clear=False,
        ), mock.patch.object(integration, "_request_internal_api_json", _fake_request):
            blocked, reason = integration.check_onboarding_blocklist(
                discord_user_id=265152027863023617,
                twitch_login="MasterIOFPS",
            )

        self.assertTrue(blocked)
        self.assertEqual(reason, "twitch_raid_blacklist fuer masteriofps")


if __name__ == "__main__":
    unittest.main()
