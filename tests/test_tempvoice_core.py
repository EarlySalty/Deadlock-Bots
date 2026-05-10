from __future__ import annotations

import unittest
import unittest.mock as mock

from cogs.tempvoice.core import TempVoiceCore


class TempVoiceCoreTests(unittest.IsolatedAsyncioTestCase):
    async def test_set_lane_template_updates_limit_when_rename_is_queued(self) -> None:
        bot = mock.Mock()
        bot.queue_channel_rename = mock.AsyncMock()
        core = TempVoiceCore(bot)
        core._persist_lane_base = mock.AsyncMock()  # type: ignore[method-assign]

        lane = mock.Mock()
        lane.id = 1234
        lane.name = "Lane 1"
        lane.user_limit = 8
        lane.edit = mock.AsyncMock()

        await core.set_lane_template(lane, base_name="Chill 1", limit=6)

        core._persist_lane_base.assert_awaited_once_with(1234, "Chill 1")
        bot.queue_channel_rename.assert_awaited_once_with(
            1234,
            "Chill 1",
            reason="TempVoice: Template Chill 1",
        )
        lane.edit.assert_awaited_once_with(
            user_limit=6,
            reason="TempVoice: Template Chill 1",
        )


if __name__ == "__main__":
    unittest.main()
