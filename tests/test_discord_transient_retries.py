from __future__ import annotations

import unittest
import unittest.mock as mock

import discord

from cogs import clip_submission, rules_channel
from service.discord_utils import retry_discord_http


class _FakeHttpResponse:
    def __init__(
        self,
        status: int,
        *,
        reason: str,
        headers: dict[str, str] | None = None,
    ) -> None:
        self.status = status
        self.reason = reason
        self.headers = headers or {}


def _discord_server_error(*, retry_after: float | None = None) -> discord.DiscordServerError:
    headers: dict[str, str] = {}
    if retry_after is not None:
        headers["Retry-After"] = str(retry_after)
    return discord.DiscordServerError(
        _FakeHttpResponse(503, reason="Service Unavailable", headers=headers),
        {"message": "temporary outage"},
    )


def _discord_not_found() -> discord.NotFound:
    return discord.NotFound(
        _FakeHttpResponse(404, reason="Not Found"),
        {"message": "missing"},
    )


class _FakeThread:
    def __init__(self, thread_id: int, *, add_user_failures: int = 0) -> None:
        self.id = thread_id
        self.mention = f"<#{thread_id}>"
        self.deleted = False
        self.add_user_calls = 0
        self.send_calls: list[dict[str, object]] = []
        self._remaining_add_user_failures = max(0, int(add_user_failures))

    async def add_user(self, _user: object) -> None:
        self.add_user_calls += 1
        if self._remaining_add_user_failures > 0:
            self._remaining_add_user_failures -= 1
            raise _discord_server_error()

    async def delete(self) -> None:
        self.deleted = True

    async def send(self, **kwargs: object) -> None:
        self.send_calls.append(dict(kwargs))


class _FakeTextChannel:
    def __init__(self, *, private_thread: object, public_thread: object) -> None:
        self.id = 9876
        self._private_thread = private_thread
        self._public_thread = public_thread
        self.create_calls: list[dict[str, object]] = []
        self.send_calls = 0

    async def create_thread(self, **kwargs: object):
        self.create_calls.append(dict(kwargs))
        channel_type = kwargs["type"]
        target = (
            self._private_thread
            if channel_type == discord.ChannelType.private_thread
            else self._public_thread
        )
        if isinstance(target, Exception):
            raise target
        return target

    async def send(self, **_kwargs: object):
        self.send_calls += 1
        raise _discord_server_error()


class _FakeGuild:
    def __init__(self, channel: _FakeTextChannel) -> None:
        self.id = 1234
        self._channel = channel

    def get_channel(self, _channel_id: int) -> _FakeTextChannel:
        return self._channel


class _FakeUser:
    def __init__(self, user_id: int, name: str = "Tester") -> None:
        self.id = user_id
        self.name = name


class _FakeInteractionResponse:
    def __init__(self) -> None:
        self.sent_messages: list[dict[str, object]] = []
        self._done = False

    def is_done(self) -> bool:
        return self._done

    async def send_message(self, content: str, *, ephemeral: bool = False) -> None:
        self.sent_messages.append({"content": content, "ephemeral": ephemeral})
        self._done = True


class _FakeFollowup:
    def __init__(self) -> None:
        self.sent_messages: list[dict[str, object]] = []

    async def send(self, content: str, *, ephemeral: bool = False) -> None:
        self.sent_messages.append({"content": content, "ephemeral": ephemeral})


class _FakeInteraction:
    def __init__(self, guild: _FakeGuild, user: _FakeUser) -> None:
        self.guild = guild
        self.user = user
        self.response = _FakeInteractionResponse()
        self.followup = _FakeFollowup()


class DiscordTransientRetryTests(unittest.IsolatedAsyncioTestCase):
    async def test_retry_discord_http_retries_transient_server_error(self) -> None:
        attempts = 0

        async def _operation() -> str:
            nonlocal attempts
            attempts += 1
            if attempts == 1:
                raise _discord_server_error(retry_after=0.25)
            return "ok"

        sleep_mock = mock.AsyncMock()
        with mock.patch("service.discord_utils.asyncio.sleep", new=sleep_mock):
            result = await retry_discord_http(_operation)

        self.assertEqual(result, "ok")
        self.assertEqual(attempts, 2)
        sleep_mock.assert_awaited_once_with(0.25)

    async def test_retry_discord_http_does_not_retry_non_transient_error(self) -> None:
        attempts = 0

        async def _operation() -> None:
            nonlocal attempts
            attempts += 1
            raise _discord_not_found()

        sleep_mock = mock.AsyncMock()
        with mock.patch("service.discord_utils.asyncio.sleep", new=sleep_mock):
            with self.assertRaises(discord.NotFound):
                await retry_discord_http(_operation)

        self.assertEqual(attempts, 1)
        sleep_mock.assert_not_awaited()

    async def test_create_user_thread_falls_back_to_public_thread_after_transient_private_failure(
        self,
    ) -> None:
        private_thread = _FakeThread(111, add_user_failures=3)
        public_thread = _FakeThread(222)
        channel = _FakeTextChannel(
            private_thread=private_thread,
            public_thread=public_thread,
        )
        interaction = _FakeInteraction(_FakeGuild(channel), _FakeUser(42, name="Nani"))

        sleep_mock = mock.AsyncMock()
        with mock.patch.object(rules_channel.discord, "TextChannel", _FakeTextChannel):
            with mock.patch("service.discord_utils.asyncio.sleep", new=sleep_mock):
                thread = await rules_channel._create_user_thread(interaction)  # type: ignore[arg-type]

        self.assertIs(thread, public_thread)
        self.assertTrue(private_thread.deleted)
        self.assertEqual(private_thread.add_user_calls, 3)
        self.assertEqual(
            [call["type"] for call in channel.create_calls],
            [
                discord.ChannelType.private_thread,
                discord.ChannelType.public_thread,
            ],
        )
        self.assertEqual(interaction.response.sent_messages, [])

    async def test_clip_upsert_interface_returns_none_on_transient_send_failure(self) -> None:
        channel = _FakeTextChannel(
            private_thread=_FakeThread(1),
            public_thread=_FakeThread(2),
        )
        guild = _FakeGuild(channel)
        cog = object.__new__(clip_submission.ClipSubmissionCog)
        cog.bot = mock.Mock()
        cog._window_line = lambda _guild_id: "window"  # type: ignore[method-assign]
        cog._find_existing_interface_message = mock.AsyncMock(return_value=None)  # type: ignore[method-assign]

        sleep_mock = mock.AsyncMock()
        with mock.patch.object(clip_submission.discord, "TextChannel", _FakeTextChannel):
            with mock.patch.object(clip_submission, "pv_get_latest", return_value=None):
                with mock.patch.object(clip_submission, "pv_upsert_single") as upsert_mock:
                    with mock.patch("service.discord_utils.asyncio.sleep", new=sleep_mock):
                        result = await clip_submission.ClipSubmissionCog.upsert_interface(
                            cog,
                            guild,  # type: ignore[arg-type]
                        )

        self.assertIsNone(result)
        self.assertEqual(channel.send_calls, 3)
        upsert_mock.assert_not_called()


if __name__ == "__main__":
    unittest.main()
