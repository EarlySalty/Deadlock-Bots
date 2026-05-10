from __future__ import annotations

import unittest
import unittest.mock as mock

from cogs.rename_manager import RenameManagerCog


class RenameManagerCogTests(unittest.IsolatedAsyncioTestCase):
    async def test_set_request_pending_requeues_throttled_request_to_queue_tail(self) -> None:
        cog = RenameManagerCog(mock.Mock())
        execute_mock = mock.AsyncMock()

        with mock.patch("cogs.rename_manager.db.execute_async", new=execute_mock):
            await cog._set_request_pending(
                42,
                last_error="Channel throttle active (120.0s remaining)",
                increment_retry=False,
            )

        sql, params = execute_mock.await_args.args
        self.assertIn("created_at=CURRENT_TIMESTAMP", sql)
        self.assertNotIn("retry_count=retry_count+1", sql)
        self.assertEqual(
            params,
            ("Channel throttle active (120.0s remaining)", 42),
        )
