from __future__ import annotations

import json
import unittest
from typing import Any
from unittest.mock import patch

import discord

from service import master_broker as master_broker_module
from service.master_broker import (
    _IDEMPOTENCY_HEADER,
    _INTERNAL_TOKEN_HEADER,
    MasterBroker,
)
from service.scam_revoke import ScamRevokeView


class _FakeRequest:
    def __init__(
        self,
        payload: dict[str, Any],
        *,
        headers: dict[str, str],
        remote: str = "127.0.0.1",
        query: dict[str, str] | None = None,
    ) -> None:
        self._payload = payload
        self.headers = headers
        self.remote = remote
        self.transport = None
        self.query = dict(query or {})

    async def json(self) -> dict[str, Any]:
        return dict(self._payload)


class _FakeDiscordHttp:
    def __init__(
        self,
        payload: dict[str, Any] | None = None,
        error: Exception | None = None,
    ) -> None:
        self.payload = payload or {}
        self.error = error
        self.calls: list[tuple[int, int]] = []

    async def get_message(self, channel_id: int, message_id: int) -> dict[str, Any]:
        self.calls.append((channel_id, message_id))
        if self.error is not None:
            raise self.error
        return self.payload


class _FakeHttpResponse:
    status = 404
    reason = "Not Found"
    headers: dict[str, str] = {}


class _FakeMessage:
    def __init__(self, message_id: int) -> None:
        self.id = message_id
        self.edit_calls: list[dict[str, Any]] = []

    async def edit(self, **kwargs: Any) -> None:
        self.edit_calls.append(dict(kwargs))


class _FakeChannel:
    def __init__(
        self,
        channel_id: int,
        *,
        message: _FakeMessage | None = None,
        name: str = "fake-channel",
        category_id: int | None = None,
        last_message_id: int | None = None,
    ) -> None:
        self.id = channel_id
        self.name = name
        self.category_id = category_id
        self.last_message_id = last_message_id
        self.sent_calls: list[dict[str, Any]] = []
        self._message = message or _FakeMessage(4321)
        self.fetch_calls: list[int] = []
        self.deleted = False

    async def send(self, **kwargs: Any) -> _FakeMessage:
        self.sent_calls.append(dict(kwargs))
        return self._message

    async def fetch_message(self, message_id: int) -> _FakeMessage | None:
        self.fetch_calls.append(message_id)
        if self._message.id == message_id:
            return self._message
        return None

    async def delete(self) -> None:
        self.deleted = True


class _FakeDmUser:
    def __init__(self, user_id: int, *, dm_channel: _FakeChannel | None = None) -> None:
        self.id = user_id
        self.dm_channel = dm_channel or _FakeChannel(9000 + user_id)

    async def create_dm(self) -> _FakeChannel:
        return self.dm_channel


class _FakePermissions:
    def __init__(
        self,
        *,
        administrator: bool = False,
        manage_guild: bool = False,
        manage_channels: bool = False,
    ) -> None:
        self.administrator = administrator
        self.manage_guild = manage_guild
        self.manage_channels = manage_channels


class _FakeRole:
    def __init__(
        self,
        role_id: int,
        *,
        name: str | None = None,
        is_default: bool = False,
        mentionable: bool = False,
        permissions: _FakePermissions | None = None,
    ) -> None:
        self.id = role_id
        self.name = name or f"role-{role_id}"
        self._is_default = is_default
        self.mentionable = mentionable
        self.permissions = permissions or _FakePermissions()
        self.position = 0
        self.members: list[Any] = []

    def is_default(self) -> bool:
        return self._is_default


class _FakeMember:
    def __init__(self, user_id: int) -> None:
        self.id = user_id


class _FakeCategory:
    def __init__(self, channel_id: int, guild: _FakeGuild) -> None:
        self.id = channel_id
        self.guild = guild


class _FakeGuild:
    def __init__(
        self,
        guild_id: int = 1,
        *,
        default_role: _FakeRole | None = None,
        roles: list[_FakeRole] | None = None,
        members: list[_FakeMember] | None = None,
        me: _FakeMember | None = None,
    ) -> None:
        self.id = guild_id
        self.created_channels: list[dict[str, Any]] = []
        self.created_roles: list[dict[str, Any]] = []
        self.default_role = default_role or _FakeRole(guild_id, is_default=True)
        self.roles = roles or [self.default_role]
        self._roles = {int(role.id): role for role in self.roles}
        self._members = {int(member.id): member for member in members or []}
        self._channels: dict[int, Any] = {}
        self.me = me
        self.chunked = True

    async def chunk(self) -> None:
        self.chunked = True

    def add_channel(self, channel: Any) -> None:
        self._channels[int(channel.id)] = channel

    def get_channel(self, channel_id: int) -> Any | None:
        return self._channels.get(int(channel_id))

    async def create_text_channel(
        self, *, name: str, category: _FakeCategory, topic: str | None = None, **kwargs: Any
    ) -> _FakeChannel:
        channel = _FakeChannel(7000 + len(self.created_channels) + 1)
        self.created_channels.append(
            {
                "name": name,
                "category": category,
                "topic": topic,
                "kwargs": dict(kwargs),
                "channel": channel,
            }
        )
        return channel

    async def create_role(
        self, *, name: str, mentionable: bool = False, reason: str | None = None
    ) -> _FakeRole:
        role = _FakeRole(8000 + len(self.created_roles) + 1, name=name, mentionable=mentionable)
        self.roles.append(role)
        self._roles[int(role.id)] = role
        self.created_roles.append(
            {
                "name": name,
                "mentionable": mentionable,
                "reason": reason,
                "role": role,
            }
        )
        return role

    def get_member(self, user_id: int) -> _FakeMember | None:
        return self._members.get(int(user_id))

    def get_role(self, role_id: int) -> _FakeRole | None:
        return self._roles.get(int(role_id))


class _TrackingTestView(discord.ui.View):
    def __init__(self) -> None:
        super().__init__(timeout=None)
        self.bound_channel_id: int | None = None
        self.bound_message_id: int | None = None
        self.add_item(
            discord.ui.Button(
                label="Track",
                style=discord.ButtonStyle.primary,
                custom_id="tests:track",
            )
        )

    def bind_to_message(self, *, channel_id: int, message_id: int) -> None:
        self.bound_channel_id = channel_id
        self.bound_message_id = message_id


class _FakeBot:
    def __init__(
        self,
        *,
        channel: Any | None = None,
        user: _FakeDmUser | None = None,
        guilds: list[Any] | None = None,
    ) -> None:
        self._channels: dict[int, Any] = {}
        if channel is not None:
            self._channels[int(channel.id)] = channel
        self._users: dict[int, _FakeDmUser] = {}
        if user is not None:
            self._users[int(user.id)] = user
        self.guilds = list(guilds or [])
        self.added_views: list[tuple[discord.ui.View, int | None]] = []
        self.http: Any = None

    def get_guild(self, guild_id: int) -> Any | None:
        for guild in self.guilds:
            if int(getattr(guild, "id", 0)) == int(guild_id):
                return guild
        return None

    def add_channel(self, channel: Any) -> None:
        self._channels[int(channel.id)] = channel

    def add_user(self, user: _FakeDmUser) -> None:
        self._users[int(user.id)] = user

    def get_channel(self, channel_id: int) -> Any | None:
        return self._channels.get(int(channel_id))

    async def fetch_channel(self, channel_id: int) -> Any | None:
        return self.get_channel(channel_id)

    def get_user(self, user_id: int) -> _FakeDmUser | None:
        return self._users.get(int(user_id))

    async def fetch_user(self, user_id: int) -> _FakeDmUser | None:
        return self.get_user(user_id)

    def add_view(self, view: discord.ui.View, *, message_id: int | None = None) -> None:
        self.added_views.append((view, message_id))

    def is_ready(self) -> bool:
        return True


class MasterBrokerTests(unittest.IsolatedAsyncioTestCase):
    def _headers(self, idempotency_key: str = "req-1") -> dict[str, str]:
        return {
            _INTERNAL_TOKEN_HEADER: "secret-token",
            _IDEMPOTENCY_HEADER: idempotency_key,
        }

    @staticmethod
    def _payload(response) -> dict[str, Any]:
        return json.loads(response.text)

    async def test_message_reactions_route_is_registered(self) -> None:
        broker = MasterBroker(
            _FakeBot(),
            token=self._headers()[_INTERNAL_TOKEN_HEADER],
            port=0,
        )
        try:
            await broker.start()
            assert broker._runner is not None
            routes = {
                (route.method, route.resource.canonical)
                for route in broker._runner.app.router.routes()
            }
            self.assertIn(
                ("GET", "/internal/master/v1/discord/message-reactions"),
                routes,
            )
        finally:
            await broker.stop()

    async def test_message_reactions_requires_internal_token(self) -> None:
        bot = _FakeBot()
        bot.http = _FakeDiscordHttp()
        broker = MasterBroker(bot, token=self._headers()[_INTERNAL_TOKEN_HEADER])
        request = _FakeRequest(
            {},
            headers={},
            query={"channel_id": "123", "message_id": "456"},
        )

        response = await broker._handle_message_reactions(request)

        self.assertEqual(response.status, 401)
        self.assertEqual(self._payload(response)["error"]["code"], "unauthorized")
        self.assertEqual(bot.http.calls, [])

    async def test_message_reactions_returns_unicode_and_custom_emoji(self) -> None:
        channel_id = 123456789012345678
        message_id = 987654321098765432
        custom_emoji_id = 112233445566778899
        bot = _FakeBot()
        bot.http = _FakeDiscordHttp(
            {
                "reactions": [
                    {"emoji": {"id": None, "name": "👍"}, "count": 2},
                    {
                        "emoji": {"id": custom_emoji_id, "name": "party"},
                        "count": 3,
                    },
                ]
            }
        )
        broker = MasterBroker(bot, token=self._headers()[_INTERNAL_TOKEN_HEADER])
        request = _FakeRequest(
            {},
            headers=self._headers(),
            query={"channel_id": str(channel_id), "message_id": str(message_id)},
        )

        response = await broker._handle_message_reactions(request)

        self.assertEqual(response.status, 200)
        self.assertEqual(
            self._payload(response),
            {
                "found": True,
                "reactions": [
                    {"emoji": "👍", "count": 2},
                    {"emoji": f"party:{custom_emoji_id}", "count": 3},
                ],
            },
        )
        self.assertEqual(bot.http.calls, [(channel_id, message_id)])

    async def test_message_reactions_maps_discord_404_to_not_found(self) -> None:
        not_found = discord.NotFound(
            _FakeHttpResponse(),
            {"message": "Unknown Message", "code": 10008},
        )
        bot = _FakeBot()
        bot.http = _FakeDiscordHttp(error=not_found)
        broker = MasterBroker(bot, token=self._headers()[_INTERNAL_TOKEN_HEADER])
        request = _FakeRequest(
            {},
            headers=self._headers(),
            query={"channel_id": "123", "message_id": "456"},
        )

        response = await broker._handle_message_reactions(request)

        self.assertEqual(response.status, 200)
        self.assertEqual(self._payload(response), {"found": False, "reactions": []})

    async def test_message_reactions_maps_timeout_to_bad_gateway(self) -> None:
        bot = _FakeBot()
        bot.http = _FakeDiscordHttp(error=TimeoutError())
        broker = MasterBroker(bot, token=self._headers()[_INTERNAL_TOKEN_HEADER])
        request = _FakeRequest(
            {},
            headers=self._headers(),
            query={"channel_id": "123", "message_id": "456"},
        )

        response = await broker._handle_message_reactions(request)

        self.assertEqual(response.status, 502)
        self.assertEqual(self._payload(response), {"error": "Discord request failed"})

    async def test_send_message_supports_dm_by_user_id(self) -> None:
        user = _FakeDmUser(123)
        bot = _FakeBot(user=user)
        broker = MasterBroker(bot, token="secret-token")
        request = _FakeRequest(
            {
                "user_id": 123,
                "content": "Direktnachricht",
            },
            headers=self._headers("req-dm"),
        )

        response = await broker._handle_send_message(request)

        self.assertEqual(response.status, 200)
        body = self._payload(response)
        self.assertEqual(body["result"]["user_id"], 123)
        self.assertEqual(body["result"]["channel_id"], user.dm_channel.id)
        self.assertEqual(user.dm_channel.sent_calls[0]["content"], "Direktnachricht")

    async def test_channel_info_rejects_non_loopback(self) -> None:
        guild = _FakeGuild()
        channel = _FakeChannel(7777, name="ticket-123")
        guild.add_channel(channel)
        bot = _FakeBot(guilds=[guild])
        broker = MasterBroker(bot, token="secret-token")
        request = _FakeRequest(
            {},
            headers=self._headers("req-channel-info-reject"),
            remote="10.0.0.5",
            query={"channel_id": "7777"},
        )

        response = await broker._handle_channel_info(request)

        self.assertEqual(response.status, 403)
        body = self._payload(response)
        self.assertEqual(body["error"]["code"], "forbidden")

    async def test_channel_info_returns_metadata(self) -> None:
        guild = _FakeGuild()
        channel = _FakeChannel(
            7777,
            name="ticket-123",
            category_id=555,
            last_message_id=999888,
        )
        guild.add_channel(channel)
        bot = _FakeBot(guilds=[guild])
        broker = MasterBroker(bot, token="secret-token")
        request = _FakeRequest(
            {},
            headers=self._headers("req-channel-info-ok"),
            query={"channel_id": "7777"},
        )

        response = await broker._handle_channel_info(request)

        self.assertEqual(response.status, 200)
        body = self._payload(response)
        self.assertEqual(body["ok"], True)
        self.assertEqual(body["channel_id"], "7777")
        self.assertEqual(body["name"], "ticket-123")
        self.assertEqual(body["parent_id"], "555")
        self.assertEqual(body["last_message_id"], "999888")

    async def test_channel_info_unknown_channel_returns_404(self) -> None:
        guild = _FakeGuild()
        bot = _FakeBot(guilds=[guild])
        broker = MasterBroker(bot, token="secret-token")
        request = _FakeRequest(
            {},
            headers=self._headers("req-channel-info-404"),
            query={"channel_id": "12345"},
        )

        response = await broker._handle_channel_info(request)

        self.assertEqual(response.status, 404)
        body = self._payload(response)
        self.assertEqual(body["error"]["code"], "not_found")
        self.assertIn("12345", body["error"]["message"])

    async def test_create_and_delete_channel_endpoints(self) -> None:
        guild = _FakeGuild()
        category = _FakeCategory(555, guild)
        bot = _FakeBot(channel=category)
        broker = MasterBroker(bot, token="secret-token")

        create_request = _FakeRequest(
            {
                "name": "match-alpha-vs-bravo",
                "category_id": 555,
                "topic": "Test Topic",
            },
            headers=self._headers("req-create"),
        )
        create_response = await broker._handle_create_channel(create_request)

        self.assertEqual(create_response.status, 200)
        create_body = self._payload(create_response)
        created_channel = guild.created_channels[0]["channel"]
        self.assertEqual(create_body["result"]["channel_id"], created_channel.id)
        self.assertEqual(guild.created_channels[0]["name"], "match-alpha-vs-bravo")
        self.assertEqual(guild.created_channels[0]["topic"], "Test Topic")
        self.assertNotIn("overwrites", guild.created_channels[0]["kwargs"])

        bot.add_channel(created_channel)
        delete_request = _FakeRequest(
            {
                "channel_id": created_channel.id,
            },
            headers=self._headers("req-delete"),
        )
        delete_response = await broker._handle_delete_channel(delete_request)

        self.assertEqual(delete_response.status, 200)
        delete_body = self._payload(delete_response)
        self.assertEqual(delete_body["result"]["channel_id"], created_channel.id)
        self.assertTrue(created_channel.deleted)

    async def test_create_ticket_channel_sets_private_overwrites(self) -> None:
        everyone = _FakeRole(1, is_default=True)
        staff_role = _FakeRole(22)
        admin_role = _FakeRole(33, permissions=_FakePermissions(administrator=True))
        normal_role = _FakeRole(44)
        owner = _FakeMember(123)
        bot_member = _FakeMember(999)
        guild = _FakeGuild(
            default_role=everyone,
            roles=[everyone, staff_role, admin_role, normal_role],
            members=[owner, bot_member],
            me=bot_member,
        )
        category = _FakeCategory(555, guild)
        bot = _FakeBot(channel=category)
        broker = MasterBroker(bot, token="secret-token")
        request = _FakeRequest(
            {
                "name": "beta-ticket-123",
                "category_id": 555,
                "ticket_owner_id": 123,
            },
            headers=self._headers("req-create-ticket"),
        )

        with patch.object(
            master_broker_module.welcome_base,
            "WELCOME_DM_TEST_ROLE_IDS",
            (staff_role.id,),
        ):
            response = await broker._handle_create_channel(request)

        self.assertEqual(response.status, 200)
        overwrites = guild.created_channels[0]["kwargs"]["overwrites"]
        self.assertIs(overwrites[everyone].view_channel, False)
        self.assertIs(overwrites[owner].view_channel, True)
        self.assertIs(overwrites[owner].send_messages, True)
        self.assertIs(overwrites[owner].read_message_history, True)
        self.assertIs(overwrites[owner].attach_files, True)
        self.assertIs(overwrites[owner].embed_links, True)
        self.assertIs(overwrites[owner].add_reactions, True)
        self.assertIs(overwrites[bot_member].manage_channels, True)
        self.assertIs(overwrites[bot_member].manage_messages, True)
        self.assertIs(overwrites[staff_role].manage_messages, True)
        self.assertIs(overwrites[admin_role].manage_channels, True)
        self.assertNotIn(normal_role, overwrites)

    def test_ticket_overwrites_skip_missing_owner_but_keep_channel_private(self) -> None:
        everyone = _FakeRole(1, is_default=True)
        guild = _FakeGuild(default_role=everyone)

        overwrites = master_broker_module._build_ticket_overwrites(guild, 404)

        self.assertIs(overwrites[everyone].view_channel, False)
        self.assertEqual(len(overwrites), 1)

    async def test_create_role_returns_string_role_id_and_replays_idempotently(self) -> None:
        guild = _FakeGuild(guild_id=123)
        bot = _FakeBot(guilds=[guild])
        broker = MasterBroker(bot, token="secret-token")
        payload = {
            "guild_id": "123",
            "name": "foo ist live",
            "mentionable": True,
            "reason": "Auto-created Twitch live ping role for foo",
        }
        request = _FakeRequest(payload, headers=self._headers("req-create-role"))

        response = await broker._handle_create_role(request)

        self.assertEqual(response.status, 200)
        body = self._payload(response)
        self.assertTrue(body["ok"])
        self.assertEqual(body["result"], {"role_id": "8001"})
        self.assertEqual(len(guild.created_roles), 1)
        self.assertEqual(guild.created_roles[0]["name"], "foo ist live")
        self.assertIs(guild.created_roles[0]["mentionable"], True)
        self.assertEqual(
            guild.created_roles[0]["reason"],
            "Auto-created Twitch live ping role for foo",
        )

        replay = _FakeRequest(payload, headers=self._headers("req-create-role"))
        replay_response = await broker._handle_create_role(replay)

        self.assertEqual(replay_response.status, 200)
        replay_body = self._payload(replay_response)
        self.assertTrue(replay_body["cached"])
        self.assertEqual(replay_body["result"], {"role_id": "8001"})
        self.assertEqual(len(guild.created_roles), 1)

    async def test_send_rich_message_builds_link_button_and_allowed_mentions(self) -> None:
        channel = _FakeChannel(111)
        bot = _FakeBot(channel=channel)
        broker = MasterBroker(bot, token="secret-token")
        request = _FakeRequest(
            {
                "channel_id": 111,
                "content": "  <@&55> Stream ist live  ",
                "embed": {"title": "Now Live"},
                "allowed_role_ids": [55],
                "view_spec": {
                    "type": "link_button",
                    "label": "Zum VOD",
                    "url": "https://www.twitch.tv/example",
                },
            },
            headers=self._headers(),
        )

        response = await broker._handle_send_rich_message(request)

        self.assertEqual(response.status, 200)
        body = self._payload(response)
        self.assertTrue(body["ok"])
        self.assertEqual(body["result"]["message_id"], "4321")
        self.assertEqual(len(channel.sent_calls), 1)
        sent = channel.sent_calls[0]
        self.assertEqual(sent["content"], "<@&55> Stream ist live")
        self.assertIsInstance(sent["embed"], discord.Embed)
        self.assertIsInstance(sent["view"], discord.ui.View)
        self.assertEqual(sent["view"].children[0].label, "Zum VOD")
        self.assertEqual(sent["view"].children[0].url, "https://www.twitch.tv/example")
        self.assertEqual([role.id for role in sent["allowed_mentions"].roles], [55])
        self.assertEqual(bot.added_views, [])

    async def test_send_rich_message_uses_tracking_view_resolver_and_registers_view(self) -> None:
        channel = _FakeChannel(222)
        bot = _FakeBot(channel=channel)

        async def _resolve(spec: dict[str, Any]) -> discord.ui.View:
            self.assertEqual(spec["type"], "twitch_live_tracking")
            return _TrackingTestView()

        bot.resolve_master_broker_view_spec = _resolve  # type: ignore[attr-defined]
        broker = MasterBroker(bot, token="secret-token")
        request = _FakeRequest(
            {
                "channel_id": 222,
                "embed": {"title": "Tracking"},
                "view_spec": {
                    "type": "twitch_live_tracking",
                    "streamer_login": "example",
                    "tracking_token": "abc123",
                },
            },
            headers=self._headers(),
        )

        response = await broker._handle_send_rich_message(request)

        self.assertEqual(response.status, 200)
        self.assertEqual(len(channel.sent_calls), 1)
        self.assertEqual(len(bot.added_views), 1)
        registered_view, registered_message_id = bot.added_views[0]
        self.assertEqual(registered_message_id, 4321)
        self.assertIsInstance(registered_view, _TrackingTestView)
        self.assertEqual(registered_view.bound_channel_id, 222)
        self.assertEqual(registered_view.bound_message_id, 4321)

    async def test_send_rich_message_requires_tracking_view_resolver(self) -> None:
        channel = _FakeChannel(333)
        broker = MasterBroker(_FakeBot(channel=channel), token="secret-token")
        request = _FakeRequest(
            {
                "channel_id": 333,
                "embed": {"title": "Tracking"},
                "view_spec": {"type": "twitch_live_tracking"},
            },
            headers=self._headers(),
        )

        response = await broker._handle_send_rich_message(request)

        self.assertEqual(response.status, 503)
        body = self._payload(response)
        self.assertEqual(body["error"]["code"], "view_resolver_unavailable")
        self.assertEqual(channel.sent_calls, [])

    async def test_edit_rich_message_fetches_and_edits_message(self) -> None:
        message = _FakeMessage(9876)
        channel = _FakeChannel(444, message=message)
        bot = _FakeBot(channel=channel)
        bot.resolve_master_broker_view_spec = lambda spec: _TrackingTestView()  # type: ignore[attr-defined]
        broker = MasterBroker(bot, token="secret-token")
        request = _FakeRequest(
            {
                "channel_id": 444,
                "message_id": 9876,
                "content": "Offline",
                "embed": {"title": "Offline"},
                "allowed_role_ids": [77],
                "view_spec": {
                    "type": "twitch_live_tracking",
                    "streamer_login": "example",
                    "tracking_token": "zzz",
                },
            },
            headers=self._headers("req-edit"),
        )

        response = await broker._handle_edit_rich_message(request)

        self.assertEqual(response.status, 200)
        self.assertEqual(channel.fetch_calls, [9876])
        self.assertEqual(len(message.edit_calls), 1)
        edit_call = message.edit_calls[0]
        self.assertEqual(edit_call["content"], "Offline")
        self.assertIsInstance(edit_call["embed"], discord.Embed)
        self.assertEqual([role.id for role in edit_call["allowed_mentions"].roles], [77])
        self.assertEqual(len(bot.added_views), 1)
        registered_view, registered_message_id = bot.added_views[0]
        self.assertEqual(registered_message_id, 9876)
        self.assertIsInstance(registered_view, _TrackingTestView)
        self.assertEqual(registered_view.bound_channel_id, 444)
        self.assertEqual(registered_view.bound_message_id, 9876)

    def _scam_spec(self, **overrides: Any) -> dict[str, Any]:
        spec: dict[str, Any] = {
            "type": "scam_revoke",
            "verdict_id": 99,
            "channel_login": "earlysalty",
            "chatter_login": "sophiaa_star",
            "action_taken": "banned",
        }
        spec.update(overrides)
        for key, value in list(spec.items()):
            if value is None:
                del spec[key]
        return spec

    async def _send_scam(self, **overrides: Any):
        channel = _FakeChannel(666)
        bot = _FakeBot(channel=channel)
        broker = MasterBroker(bot, token="secret-token")
        request = _FakeRequest(
            {
                "channel_id": 666,
                "embed": {"title": "Scam erkannt"},
                "view_spec": self._scam_spec(**overrides),
            },
            headers=self._headers("req-scam"),
        )
        response = await broker._handle_send_rich_message(request)
        return response, channel, bot

    async def test_send_rich_message_builds_scam_revoke_view_and_registers(self) -> None:
        response, channel, bot = await self._send_scam()

        self.assertEqual(response.status, 200)
        self.assertEqual(len(channel.sent_calls), 1)
        sent_view = channel.sent_calls[0]["view"]
        self.assertIsInstance(sent_view, ScamRevokeView)
        self.assertEqual(sent_view.verdict_id, 99)
        self.assertEqual(sent_view.children[0].label, "Rückgängig")

        self.assertEqual(len(bot.added_views), 1)
        registered_view, registered_message_id = bot.added_views[0]
        self.assertIs(registered_view, sent_view)
        self.assertEqual(registered_message_id, 4321)
        self.assertEqual(registered_view.channel_id, 666)
        self.assertEqual(registered_view.message_id, 4321)

    async def test_send_rich_message_rejects_scam_revoke_without_verdict_id(self) -> None:
        response, channel, bot = await self._send_scam(verdict_id=None)

        self.assertEqual(response.status, 400)
        body = self._payload(response)
        self.assertEqual(body["error"]["code"], "bad_request")
        self.assertIn("view_spec.verdict_id", body["error"]["message"])
        self.assertEqual(channel.sent_calls, [])
        self.assertEqual(bot.added_views, [])

    async def test_send_rich_message_rejects_scam_revoke_non_positive_verdict_id(self) -> None:
        response, channel, _ = await self._send_scam(verdict_id=0)

        self.assertEqual(response.status, 400)
        body = self._payload(response)
        self.assertIn("view_spec.verdict_id", body["error"]["message"])
        self.assertEqual(channel.sent_calls, [])

    async def test_send_rich_message_rejects_scam_revoke_missing_channel_login(self) -> None:
        response, channel, _ = await self._send_scam(channel_login=None)

        self.assertEqual(response.status, 400)
        body = self._payload(response)
        self.assertIn("view_spec.channel_login", body["error"]["message"])
        self.assertEqual(channel.sent_calls, [])

    async def test_send_rich_message_rejects_scam_revoke_missing_chatter_login(self) -> None:
        response, channel, _ = await self._send_scam(chatter_login=None)

        self.assertEqual(response.status, 400)
        body = self._payload(response)
        self.assertIn("view_spec.chatter_login", body["error"]["message"])
        self.assertEqual(channel.sent_calls, [])

    async def test_send_rich_message_rejects_scam_revoke_missing_action_taken(self) -> None:
        response, channel, _ = await self._send_scam(action_taken=None)

        self.assertEqual(response.status, 400)
        body = self._payload(response)
        self.assertIn("view_spec.action_taken", body["error"]["message"])
        self.assertEqual(channel.sent_calls, [])

    async def test_send_rich_message_rejects_invalid_view_spec_type(self) -> None:
        channel = _FakeChannel(555)
        broker = MasterBroker(_FakeBot(channel=channel), token="secret-token")
        request = _FakeRequest(
            {
                "channel_id": 555,
                "embed": {"title": "Invalid"},
                "view_spec": {"type": "unknown"},
            },
            headers=self._headers("req-invalid"),
        )

        response = await broker._handle_send_rich_message(request)

        self.assertEqual(response.status, 400)
        body = self._payload(response)
        self.assertEqual(body["error"]["code"], "bad_request")
        self.assertIn("view_spec.type", body["error"]["message"])


if __name__ == "__main__":
    unittest.main()
