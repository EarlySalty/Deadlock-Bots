from __future__ import annotations

import os
import unittest
import unittest.mock as mock
from typing import Any

import discord

from cogs.twitch.live_bridge import (
    TWITCH_INTERNAL_API_BASE_PATH,
    TWITCH_INTERNAL_TOKEN_HEADER,
    TwitchLiveBridgeApiError,
    TwitchLiveBridgeCog,
    TwitchLiveInternalApiClient,
    TwitchLiveTrackingView,
)


class _FakeResponse:
    def __init__(self, *, status: int, text: str) -> None:
        self.status = int(status)
        self._text = text
        self.released = False

    async def text(self) -> str:
        return self._text

    def release(self) -> None:
        self.released = True


class _FakeSession:
    def __init__(self, responses: list[_FakeResponse]) -> None:
        self._responses = list(responses)
        self.calls: list[dict[str, Any]] = []
        self.closed = False

    async def request(self, method: str, url: str, **kwargs: Any) -> _FakeResponse:
        self.calls.append({"method": method, "url": url, "kwargs": kwargs})
        return self._responses.pop(0)

    async def close(self) -> None:
        self.closed = True


class _FakeApiClient:
    def __init__(
        self,
        announcements: list[dict[str, Any]] | None = None,
        *,
        failures_before_success: int = 0,
    ) -> None:
        self.announcements = list(announcements or [])
        self.click_calls: list[dict[str, Any]] = []
        self.closed = False
        self.failures_before_success = max(0, int(failures_before_success))
        self.fetch_calls = 0

    async def get_active_live_announcements(self) -> list[dict[str, Any]]:
        self.fetch_calls += 1
        if self.failures_before_success > 0:
            self.failures_before_success -= 1
            raise RuntimeError("temporary outage")
        return list(self.announcements)

    async def record_live_link_click(self, **kwargs: Any) -> dict[str, Any]:
        self.click_calls.append(dict(kwargs))
        return {"ok": True}

    async def close(self) -> None:
        self.closed = True


class _FakeBot:
    def __init__(self) -> None:
        self.added_views: list[tuple[discord.ui.View, int | None]] = []

    def add_view(self, view: discord.ui.View, *, message_id: int | None = None) -> None:
        self.added_views.append((view, message_id))


class _FakeUser:
    def __init__(self, user_id: int, label: str = "Viewer One") -> None:
        self.id = user_id
        self._label = label

    def __str__(self) -> str:
        return self._label


class _FakeMessage:
    def __init__(self, message_id: int) -> None:
        self.id = message_id


class _FakeInteractionResponse:
    def __init__(self) -> None:
        self.defer_calls: list[dict[str, Any]] = []
        self.sent_messages: list[dict[str, Any]] = []
        self._done = False

    def is_done(self) -> bool:
        return self._done

    async def defer(self, *, ephemeral: bool = False) -> None:
        self.defer_calls.append({"ephemeral": ephemeral})
        self._done = True

    async def send_message(
        self,
        content: str,
        *,
        view: discord.ui.View | None = None,
        ephemeral: bool = False,
    ) -> None:
        self.sent_messages.append({"content": content, "view": view, "ephemeral": ephemeral})
        self._done = True


class _FakeFollowup:
    def __init__(self) -> None:
        self.sent_messages: list[dict[str, Any]] = []

    async def send(
        self,
        content: str,
        *,
        view: discord.ui.View | None = None,
        ephemeral: bool = False,
    ) -> None:
        self.sent_messages.append({"content": content, "view": view, "ephemeral": ephemeral})


class _FakeInteraction:
    def __init__(
        self,
        *,
        interaction_id: int,
        user: _FakeUser,
        guild_id: int | None,
        channel_id: int | None,
        message_id: int | None,
    ) -> None:
        self.id = interaction_id
        self.user = user
        self.guild_id = guild_id
        self.channel_id = channel_id
        self.message = _FakeMessage(message_id) if message_id is not None else None
        self.response = _FakeInteractionResponse()
        self.followup = _FakeFollowup()


class TwitchLiveInternalApiClientTests(unittest.IsolatedAsyncioTestCase):
    async def test_internal_api_client_calls_expected_live_endpoints(self) -> None:
        session = _FakeSession(
            [
                _FakeResponse(
                    status=200,
                    text=(
                        '[{"streamer_login":"partner_one","message_id":123,'
                        '"tracking_token":"deadbeef1234",'
                        '"referral_url":"https://www.twitch.tv/partner_one?ref=DE-Deadlock-Discord",'
                        '"button_label":"Jetzt reinsehen","channel_id":456}]'
                    ),
                ),
                _FakeResponse(status=200, text='{"ok": true}'),
            ]
        )
        client = TwitchLiveInternalApiClient(
            base_url=f"http://127.0.0.1:8776{TWITCH_INTERNAL_API_BASE_PATH}",
            token="secret-token",
            session=session,  # type: ignore[arg-type]
        )

        announcements = await client.get_active_live_announcements()
        click_payload = await client.record_live_link_click(
            streamer_login="Partner_One",
            tracking_token="deadbeef1234",
            discord_user_id="12345",
            discord_username="Viewer One",
            guild_id="111",
            channel_id="222",
            message_id="333",
            source_hint="discord_button",
            idempotency_key="live-click-1",
        )

        self.assertEqual(announcements[0]["streamer_login"], "partner_one")
        self.assertEqual(click_payload, {"ok": True})
        self.assertEqual(len(session.calls), 2)
        self.assertEqual(
            session.calls[0]["url"],
            "http://127.0.0.1:8776/internal/twitch/v1/live/active-announcements",
        )
        self.assertNotIn("json", session.calls[0]["kwargs"])
        self.assertEqual(
            session.calls[0]["kwargs"]["headers"][TWITCH_INTERNAL_TOKEN_HEADER],
            "secret-token",
        )
        self.assertEqual(
            session.calls[1]["url"],
            "http://127.0.0.1:8776/internal/twitch/v1/live/link-click",
        )
        self.assertEqual(
            session.calls[1]["kwargs"]["json"],
            {
                "streamer_login": "partner_one",
                "tracking_token": "deadbeef1234",
                "discord_user_id": "12345",
                "discord_username": "Viewer One",
                "guild_id": "111",
                "channel_id": "222",
                "message_id": "333",
                "source_hint": "discord_button",
            },
        )
        self.assertEqual(
            session.calls[1]["kwargs"]["headers"]["Idempotency-Key"],
            "live-click-1",
        )

    def test_internal_api_client_rejects_non_loopback_base_url_by_default(self) -> None:
        with self.assertRaises(ValueError):
            TwitchLiveInternalApiClient(
                base_url="https://example.com/internal/twitch/v1",
                token="secret-token",
            )


class TwitchLiveBridgeCogTests(unittest.IsolatedAsyncioTestCase):
    def _build_spec(self) -> dict[str, Any]:
        return {
            "type": "twitch_live_tracking",
            "streamer_login": "partner_one",
            "tracking_token": "deadbeef1234",
            "referral_url": "https://www.twitch.tv/partner_one?ref=DE-Deadlock-Discord",
            "button_label": "Jetzt reinsehen",
        }

    async def test_cog_load_sets_resolver_and_rehydrates_active_views(self) -> None:
        bot = _FakeBot()
        api_client = _FakeApiClient(
            announcements=[
                {
                    "streamer_login": "partner_one",
                    "message_id": 123,
                    "tracking_token": "deadbeef1234",
                    "referral_url": "https://www.twitch.tv/partner_one?ref=DE-Deadlock-Discord",
                    "button_label": "Jetzt reinsehen",
                    "channel_id": 456,
                }
            ]
        )
        cog = TwitchLiveBridgeCog(bot, api_client=api_client)  # type: ignore[arg-type]

        await cog.cog_load()

        self.assertTrue(callable(bot.resolve_master_broker_view_spec))  # type: ignore[attr-defined]
        self.assertEqual(len(bot.added_views), 1)
        view, message_id = bot.added_views[0]
        self.assertEqual(message_id, 123)
        self.assertIsInstance(view, TwitchLiveTrackingView)
        self.assertEqual(view.channel_id, 456)
        self.assertEqual(view.message_id, 123)

        await cog.cog_unload()

        self.assertFalse(hasattr(bot, "resolve_master_broker_view_spec"))

    async def test_cog_load_without_api_client_does_not_install_resolver(self) -> None:
        bot = _FakeBot()
        with mock.patch.dict(os.environ, {"TWITCH_INTERNAL_API_TOKEN": ""}, clear=False):
            cog = TwitchLiveBridgeCog(bot)  # type: ignore[arg-type]
            await cog.cog_load()

        self.assertFalse(hasattr(bot, "resolve_master_broker_view_spec"))

    async def test_cog_load_retries_rehydration_after_initial_failure(self) -> None:
        bot = _FakeBot()
        api_client = _FakeApiClient(
            announcements=[
                {
                    "streamer_login": "partner_one",
                    "message_id": 123,
                    "tracking_token": "deadbeef1234",
                    "referral_url": "https://www.twitch.tv/partner_one?ref=DE-Deadlock-Discord",
                    "button_label": "Jetzt reinsehen",
                    "channel_id": 456,
                }
            ],
            failures_before_success=1,
        )
        cog = TwitchLiveBridgeCog(bot, api_client=api_client)  # type: ignore[arg-type]
        cog._restore_retry_delays = (0.0,)

        await cog.cog_load()

        self.assertIsNotNone(cog._restore_task)
        await cog._restore_task

        self.assertEqual(api_client.fetch_calls, 2)
        self.assertEqual(len(bot.added_views), 1)

    async def test_resolver_builds_tracking_view_with_expected_custom_id(self) -> None:
        bot = _FakeBot()
        cog = TwitchLiveBridgeCog(bot, api_client=_FakeApiClient())  # type: ignore[arg-type]

        view = cog.resolve_master_broker_view_spec(self._build_spec())
        self.assertIsInstance(view, TwitchLiveTrackingView)
        self.assertEqual(view.children[0].custom_id, "twitch-live:partnerone:deadbeef1234")
        self.assertEqual(view.children[0].label, "Jetzt reinsehen")

        view.bind_to_message(channel_id=456, message_id=123)
        self.assertEqual(view.channel_id, 456)
        self.assertEqual(view.message_id, 123)

    async def test_click_posts_metadata_and_sends_ephemeral_link(self) -> None:
        bot = _FakeBot()
        api_client = _FakeApiClient()
        cog = TwitchLiveBridgeCog(bot, api_client=api_client)  # type: ignore[arg-type]
        view = cog.resolve_master_broker_view_spec(self._build_spec())
        self.assertIsInstance(view, TwitchLiveTrackingView)
        view.bind_to_message(channel_id=222, message_id=333)

        interaction = _FakeInteraction(
            interaction_id=987654321,
            user=_FakeUser(12345),
            guild_id=111,
            channel_id=222,
            message_id=333,
        )

        await view.handle_click(interaction)  # type: ignore[arg-type]

        self.assertEqual(
            interaction.response.defer_calls,
            [{"ephemeral": True}],
        )
        self.assertEqual(len(api_client.click_calls), 1)
        self.assertEqual(
            api_client.click_calls[0],
            {
                "streamer_login": "partner_one",
                "tracking_token": "deadbeef1234",
                "discord_user_id": 12345,
                "discord_username": "Viewer One",
                "guild_id": 111,
                "channel_id": 222,
                "message_id": 333,
                "source_hint": "discord_button",
                "idempotency_key": "twitch-live-click-987654321",
            },
        )
        self.assertEqual(len(interaction.followup.sent_messages), 1)
        sent = interaction.followup.sent_messages[0]
        self.assertTrue(sent["ephemeral"])
        self.assertEqual(sent["content"], "Hier ist dein Twitch-Link für **partner_one**.")
        self.assertIsInstance(sent["view"], discord.ui.View)
        self.assertEqual(sent["view"].children[0].url, self._build_spec()["referral_url"])
        self.assertEqual(sent["view"].children[0].label, "Jetzt reinsehen")

    async def test_resolver_requires_configured_api_client(self) -> None:
        bot = _FakeBot()
        cog = TwitchLiveBridgeCog(bot)  # type: ignore[arg-type]
        with self.assertRaises(TwitchLiveBridgeApiError):
            cog.resolve_master_broker_view_spec(self._build_spec())


if __name__ == "__main__":
    unittest.main()
