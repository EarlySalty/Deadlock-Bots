from __future__ import annotations

import os
import unittest
from typing import Any
from unittest import mock

import discord

from service.scam_revoke import (
    ScamRevokeView,
    build_scam_revoke_custom_id,
    build_scam_revoke_view,
    post_revoke,
    revoke_url,
)

_TOKEN_ENV_NAMES = (
    "TWITCH_INTERNAL_API_TOKEN",
    "MASTER_BROKER_TOKEN",
    "MAIN_BOT_INTERNAL_TOKEN",
)


class _FakeResponse:
    def __init__(self, status: int) -> None:
        self.status = status

    async def __aenter__(self) -> _FakeResponse:
        return self

    async def __aexit__(self, *exc: Any) -> None:
        return None


class _FakeSession:
    def __init__(self, status: int = 200) -> None:
        self.status = status
        self.calls: list[dict[str, Any]] = []

    def post(self, url: str, *, json: Any, headers: dict[str, str]) -> _FakeResponse:
        self.calls.append({"url": url, "json": json, "headers": headers})
        return _FakeResponse(self.status)


class BuildScamRevokeViewTests(unittest.TestCase):
    def test_build_view_carries_contract_fields(self) -> None:
        view = build_scam_revoke_view(
            {
                "type": "scam_revoke",
                "verdict_id": 42,
                "channel_login": "earlysalty",
                "chatter_login": "sophiaa_star",
                "action_taken": "banned",
            }
        )

        self.assertIsInstance(view, ScamRevokeView)
        self.assertIsNone(view.timeout)
        self.assertEqual(view.verdict_id, 42)
        self.assertEqual(view.channel_login, "earlysalty")
        self.assertEqual(view.chatter_login, "sophiaa_star")
        self.assertEqual(view.action_taken, "banned")

        button = view.children[0]
        self.assertIsInstance(button, discord.ui.Button)
        self.assertEqual(button.label, "Rückgängig")
        self.assertEqual(button.custom_id, build_scam_revoke_custom_id(42))


class PostRevokeTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        for name in _TOKEN_ENV_NAMES:
            os.environ.pop(name, None)
        os.environ["TWITCH_INTERNAL_API_TOKEN"] = "unit-token"  # noqa: S105 (Fixture-Token, kein Secret)

    def tearDown(self) -> None:
        os.environ.pop("TWITCH_INTERNAL_API_TOKEN", None)

    async def test_post_revoke_sends_contract_payload(self) -> None:
        session = _FakeSession(status=200)

        ok = await post_revoke(7, session=session)

        self.assertTrue(ok)
        self.assertEqual(len(session.calls), 1)
        call = session.calls[0]
        self.assertEqual(call["url"], revoke_url())
        self.assertEqual(
            call["url"],
            "http://127.0.0.1:8776/internal/twitch/v1/scam-guard/revoke",
        )
        self.assertEqual(call["json"], {"verdictId": 7})
        self.assertEqual(call["headers"]["X-Internal-Token"], "unit-token")

    async def test_post_revoke_returns_false_on_error_status(self) -> None:
        session = _FakeSession(status=404)

        ok = await post_revoke(7, session=session)

        self.assertFalse(ok)

    async def test_post_revoke_returns_false_without_token(self) -> None:
        os.environ.pop("TWITCH_INTERNAL_API_TOKEN", None)
        session = _FakeSession(status=200)

        ok = await post_revoke(7, session=session)

        self.assertFalse(ok)
        self.assertEqual(session.calls, [])


class _FakeResponseProxy:
    def __init__(self) -> None:
        self.deferred = False

    def is_done(self) -> bool:
        return self.deferred

    async def defer(self, **kwargs: Any) -> None:
        self.deferred = True


class _FakeFollowup:
    def __init__(self) -> None:
        self.messages: list[dict[str, Any]] = []

    async def send(self, content: str, **kwargs: Any) -> None:
        self.messages.append({"content": content, **kwargs})


class _FakeInteraction:
    def __init__(self) -> None:
        self.response = _FakeResponseProxy()
        self.followup = _FakeFollowup()
        self.edited: list[dict[str, Any]] = []

    async def edit_original_response(self, **kwargs: Any) -> None:
        self.edited.append(dict(kwargs))


def _sample_view() -> ScamRevokeView:
    return build_scam_revoke_view(
        {
            "type": "scam_revoke",
            "verdict_id": 7,
            "channel_login": "earlysalty",
            "chatter_login": "sophiaa_star",
            "action_taken": "banned",
        }
    )


class HandleClickTests(unittest.IsolatedAsyncioTestCase):
    async def test_click_success_disables_button_and_edits_message(self) -> None:
        view = _sample_view()
        interaction = _FakeInteraction()

        with mock.patch(
            "service.scam_revoke.post_revoke",
            new=mock.AsyncMock(return_value=True),
        ):
            await view.handle_click(interaction)

        self.assertTrue(interaction.response.deferred)
        button = view.children[0]
        self.assertTrue(button.disabled)
        self.assertEqual(button.label, "Zurückgenommen")
        self.assertEqual(len(interaction.edited), 1)
        self.assertIs(interaction.edited[0]["view"], view)
        self.assertTrue(
            any("zurückgenommen" in m["content"].lower() for m in interaction.followup.messages)
        )

    async def test_click_failure_keeps_button_and_reports_error(self) -> None:
        view = _sample_view()
        interaction = _FakeInteraction()

        with mock.patch(
            "service.scam_revoke.post_revoke",
            new=mock.AsyncMock(return_value=False),
        ):
            await view.handle_click(interaction)

        button = view.children[0]
        self.assertFalse(button.disabled)
        self.assertEqual(button.label, "Rückgängig")
        self.assertEqual(len(interaction.edited), 0)
        self.assertTrue(
            any("fehlgeschlagen" in m["content"].lower() for m in interaction.followup.messages)
        )


if __name__ == "__main__":
    unittest.main()
