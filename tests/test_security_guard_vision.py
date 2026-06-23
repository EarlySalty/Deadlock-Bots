from __future__ import annotations

import unittest
from datetime import UTC, datetime, timedelta
from types import SimpleNamespace
from typing import Any
from unittest import mock

import discord

from cogs.security_guard import (
    SCAM_PROPOSAL_FOOTER_DELETE_FAILED,
    SCAM_PROPOSAL_FOOTER_DELETE_OK,
    RecentMessage,
    SecurityGuard,
)


class _FakeBot:
    def __init__(self, ai: object | None = None) -> None:
        self.user = SimpleNamespace(id=999)
        self._ai = ai

    def get_cog(self, name: str) -> object | None:
        return self._ai if name == "AIConnector" else None


class _FakeHttpResponse:
    status = 403
    reason = "Forbidden"
    headers: dict[str, str] = {}


class _FakeAttachment:
    def __init__(self, file_obj: object = "evidence-file") -> None:
        self.filename = "evidence.png"
        self.content_type = "image/png"
        self.size = 1024
        self.file_obj = file_obj
        self.message: _FakeMessage | None = None
        self.to_file_calls = 0

    async def to_file(self) -> object:
        self.to_file_calls += 1
        if self.message and self.message.deleted:
            raise RuntimeError("attachment was collected after delete")
        return self.file_obj


class _FakeMessage:
    def __init__(
        self,
        *,
        attachment: _FakeAttachment,
        delete_error: discord.HTTPException | None = None,
    ) -> None:
        self.id = 123
        self.channel = SimpleNamespace(id=456, mention="<#456>")
        self.created_at = datetime(2026, 6, 23, tzinfo=UTC)
        self.content = "suspicious"
        self.attachments = [attachment]
        self.deleted = False
        self.delete_calls = 0
        self._delete_error = delete_error
        attachment.message = self

    async def delete(self) -> None:
        self.delete_calls += 1
        if self._delete_error:
            raise self._delete_error
        self.deleted = True


class _FakeGuild:
    def __init__(self) -> None:
        self.id = 777
        self.name = "Guild"
        self.me = SimpleNamespace(guild_permissions=SimpleNamespace(moderate_members=True))


class _FakeMember:
    def __init__(self) -> None:
        self.id = 888
        self.guild = _FakeGuild()
        self.mention = "<@888>"
        self.created_at = datetime(2025, 1, 1, tzinfo=UTC)
        self.joined_at = datetime(2025, 1, 2, tzinfo=UTC)
        self.edits: list[dict[str, Any]] = []
        self.dms: list[dict[str, Any]] = []

    def __str__(self) -> str:
        return "FakeMember#0001"

    async def edit(self, **kwargs: Any) -> None:
        self.edits.append(kwargs)

    async def send(self, **kwargs: Any) -> None:
        self.dms.append(kwargs)


class _FakeModChannel:
    def __init__(self) -> None:
        self.sent: list[dict[str, Any]] = []

    async def send(self, **kwargs: Any) -> None:
        self.sent.append(kwargs)


def _recent(channel_id: int, created_at: datetime, *, image: bool = True) -> RecentMessage:
    return RecentMessage(
        message=SimpleNamespace(id=channel_id, channel=SimpleNamespace(id=channel_id)),
        channel_id=channel_id,
        created_at=created_at,
        content="",
        attachments=[object()] if image else [],
    )


class SecurityGuardImageMultiChannelTests(unittest.TestCase):
    def test_image_multi_channel_uses_five_minute_window(self) -> None:
        cog = SecurityGuard(_FakeBot())
        now = datetime(2026, 6, 23, 12, 0, tzinfo=UTC)

        msgs = [
            _recent(1, now - timedelta(seconds=10)),
            _recent(2, now - timedelta(seconds=301)),
        ]

        self.assertFalse(cog._is_image_multi_channel(msgs, now))

    def test_image_multi_channel_counts_channels_inside_window(self) -> None:
        cog = SecurityGuard(_FakeBot())
        now = datetime(2026, 6, 23, 12, 0, tzinfo=UTC)

        msgs = [
            _recent(1, now - timedelta(seconds=300)),
            _recent(2, now - timedelta(seconds=5)),
            _recent(3, now - timedelta(seconds=2), image=False),
        ]

        self.assertTrue(cog._is_image_multi_channel(msgs, now))


class SecurityGuardOpenAIVisionTests(unittest.IsolatedAsyncioTestCase):
    async def test_openai_vision_sends_data_uri_with_configured_model(self) -> None:
        ai = mock.Mock()
        ai.generate_multimodal = mock.AsyncMock(
            return_value=('{"is_scam": true, "confidence": 0.88, "reason": "scam image"}', {})
        )
        cog = SecurityGuard(_FakeBot(ai))

        is_scam, confidence, reason = await cog._openai_vision_scam(
            b"abc",
            "evidence.png",
            "image/png",
        )

        self.assertTrue(is_scam)
        self.assertEqual(confidence, 0.88)
        self.assertEqual(reason, "scam image")
        call = ai.generate_multimodal.await_args.kwargs
        self.assertEqual(call["provider"], "openai")
        self.assertEqual(call["model"], "gpt-5.4-nano")
        self.assertEqual(call["max_output_tokens"], 400)
        self.assertEqual(call["temperature"], 0)
        self.assertEqual(call["images"], ["data:image/png;base64,YWJj"])


class SecurityGuardScamProposalTests(unittest.IsolatedAsyncioTestCase):
    async def test_scam_proposal_forwards_attachment_collected_before_delete(self) -> None:
        attachment = _FakeAttachment()
        message = _FakeMessage(attachment=attachment)
        member = _FakeMember()
        mod_channel = _FakeModChannel()
        cog = SecurityGuard(_FakeBot())
        cog._persist_incident = mock.AsyncMock()  # type: ignore[method-assign]
        cog._resolve_mod_channel = mock.AsyncMock(return_value=mod_channel)  # type: ignore[method-assign]
        cog._post_public_scam_notice = mock.AsyncMock()  # type: ignore[method-assign]

        await cog._handle_scam_proposal(member, message, "reason", 0.91)

        self.assertEqual(attachment.to_file_calls, 1)
        self.assertTrue(message.deleted)
        self.assertEqual(message.delete_calls, 1)
        self.assertEqual(len(mod_channel.sent), 1)
        self.assertEqual(mod_channel.sent[0]["files"], ["evidence-file"])
        self.assertEqual(
            mod_channel.sent[0]["embed"].footer.text, SCAM_PROPOSAL_FOOTER_DELETE_OK
        )

    async def test_scam_proposal_delete_failure_still_posts_placeholder_footer(self) -> None:
        attachment = _FakeAttachment()
        delete_error = discord.Forbidden(_FakeHttpResponse(), {"message": "no"})
        message = _FakeMessage(attachment=attachment, delete_error=delete_error)
        member = _FakeMember()
        mod_channel = _FakeModChannel()
        cog = SecurityGuard(_FakeBot())
        cog._persist_incident = mock.AsyncMock()  # type: ignore[method-assign]
        cog._resolve_mod_channel = mock.AsyncMock(return_value=mod_channel)  # type: ignore[method-assign]
        cog._post_public_scam_notice = mock.AsyncMock()  # type: ignore[method-assign]

        await cog._handle_scam_proposal(member, message, "reason", 0.91)

        self.assertFalse(message.deleted)
        self.assertEqual(message.delete_calls, 1)
        self.assertEqual(mod_channel.sent[0]["files"], ["evidence-file"])
        self.assertEqual(
            mod_channel.sent[0]["embed"].footer.text, SCAM_PROPOSAL_FOOTER_DELETE_FAILED
        )


if __name__ == "__main__":
    unittest.main()
