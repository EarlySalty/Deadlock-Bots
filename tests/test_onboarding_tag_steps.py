from __future__ import annotations

import asyncio
import importlib
import sys
import types
import unittest
from importlib.util import find_spec


class _Settings(types.SimpleNamespace):
    def __getattr__(self, _name: str) -> object:
        return None


class _FakeResponse:
    def __init__(self) -> None:
        self.messages: list[dict[str, object]] = []

    async def send_message(
        self,
        content: str | None = None,
        *,
        embed: object | None = None,
        view: object | None = None,
        ephemeral: bool = False,
    ) -> None:
        self.messages.append(
            {
                "content": content,
                "embed": embed,
                "view": view,
                "ephemeral": ephemeral,
            }
        )


class _FakeTagService:
    def __init__(self) -> None:
        self.calls: list[tuple[int, str, str]] = []

    async def set_user_tag(self, user_id: int, key: str, value: str) -> None:
        self.calls.append((user_id, key, value))


class _FakeBot:
    def __init__(self, tag_service: object | None = None) -> None:
        self._tag_service = tag_service

    def get_cog(self, name: str) -> object | None:
        if name == "TagService":
            return self._tag_service
        return None


class _FakeUser:
    def __init__(self, user_id: int = 123456789) -> None:
        self.id = user_id
        self.roles: list[object] = []


class _FakeInteraction:
    def __init__(self, *, user_id: int = 123456789) -> None:
        self.user = _FakeUser(user_id=user_id)
        self.response = _FakeResponse()
        self.channel = types.SimpleNamespace(id=987654321)


@unittest.skipUnless(
    find_spec("discord") is not None, "discord.py is required for this integration test"
)
class OnboardingTagStepsTest(unittest.TestCase):
    def setUp(self) -> None:
        self._original_modules: dict[str, object] = {}
        for name in (
            "service",
            "service.config",
            "cogs.steam",
            "cogs.steam.steam_link_oauth",
            "cogs.onboarding",
        ):
            self._original_modules[name] = sys.modules.get(name)

        service_pkg = types.ModuleType("service")
        config_mod = types.ModuleType("service.config")
        config_mod.settings = _Settings(
            guild_id=1,
            verified_role_id=2,
            content_creator_role_id=3,
            public_base_url="https://legacy.example.test",
        )
        service_pkg.config = config_mod
        sys.modules["service"] = service_pkg
        sys.modules["service.config"] = config_mod

        steam_pkg = types.ModuleType("cogs.steam")
        oauth_mod = types.ModuleType("cogs.steam.steam_link_oauth")
        oauth_mod.FRIEND_CODE_LINKING_ENABLED = False
        steam_pkg.steam_link_oauth = oauth_mod
        sys.modules["cogs.steam"] = steam_pkg
        sys.modules["cogs.steam.steam_link_oauth"] = oauth_mod

        sys.modules.pop("cogs.onboarding", None)
        self.mod = importlib.import_module("cogs.onboarding")

    def tearDown(self) -> None:
        sys.modules.pop("cogs.onboarding", None)
        for name, original in self._original_modules.items():
            if original is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = original

    def test_step_order_inserts_tag_steps_before_account_link(self) -> None:
        titles = [step["title"] for step in self.mod.STEPS]

        self.assertEqual(
            titles[6:10],
            [
                "🛠️ Was der Server sonst noch so hat",
                "Voice-Ton",
                "Alter (optional)",
                "🔗 Account verknüpfen & Rang-System",
            ],
        )

    def test_voice_tone_button_saves_tag_and_advances_to_age_step(self) -> None:
        async def _run() -> None:
            tag_service = _FakeTagService()
            cog = types.SimpleNamespace(bot=_FakeBot(tag_service=tag_service))
            view = self.mod.OnboardingToneStepView(
                cog=cog, step_index=self.mod._TONE_STEP_INDEX, user_id=123456789
            )
            interaction = _FakeInteraction()
            button = next(child for child in view.children if getattr(child, "label", "") == "Banter-OK")

            await button.callback(interaction)

            self.assertEqual(tag_service.calls, [(123456789, "tone", "banter_ok")])
            self.assertEqual(len(interaction.response.messages), 1)
            message = interaction.response.messages[0]
            self.assertEqual(message["embed"].title, "Alter (optional)")
            self.assertIsInstance(message["view"], self.mod.OnboardingAgeStepView)

        asyncio.run(_run())

    def test_age_button_saves_tag_and_advances_to_account_link(self) -> None:
        async def _run() -> None:
            tag_service = _FakeTagService()
            pending_calls: list[tuple[int, int]] = []

            async def _register_pending_verify(user_id: int, channel_id: int) -> None:
                pending_calls.append((user_id, channel_id))

            cog = types.SimpleNamespace(
                bot=_FakeBot(tag_service=tag_service),
                _register_pending_verify=_register_pending_verify,
            )
            view = self.mod.OnboardingAgeStepView(
                cog=cog, step_index=self.mod._AGE_STEP_INDEX, user_id=123456789
            )
            interaction = _FakeInteraction()
            button = next(child for child in view.children if getattr(child, "label", "") == "25+")

            await button.callback(interaction)

            self.assertEqual(tag_service.calls, [(123456789, "age", "25+")])
            self.assertEqual(len(interaction.response.messages), 1)
            message = interaction.response.messages[0]
            self.assertEqual(message["embed"].title, "🔗 Account verknüpfen & Rang-System")
            self.assertIsInstance(message["view"], self.mod.OnboardingAccountLinkView)
            self.assertEqual(pending_calls, [(123456789, 987654321)])

        asyncio.run(_run())

    def test_skip_buttons_do_not_store_tags(self) -> None:
        async def _run() -> None:
            tag_service = _FakeTagService()
            pending_calls: list[tuple[int, int]] = []

            async def _register_pending_verify(user_id: int, channel_id: int) -> None:
                pending_calls.append((user_id, channel_id))

            cog = types.SimpleNamespace(
                bot=_FakeBot(tag_service=tag_service),
                _register_pending_verify=_register_pending_verify,
            )

            tone_view = self.mod.OnboardingToneStepView(
                cog=cog, step_index=self.mod._TONE_STEP_INDEX, user_id=123456789
            )
            tone_interaction = _FakeInteraction()
            tone_skip = next(
                child for child in tone_view.children if getattr(child, "label", "") == "Überspringen"
            )

            await tone_skip.callback(tone_interaction)

            self.assertEqual(tag_service.calls, [])
            tone_message = tone_interaction.response.messages[0]
            self.assertEqual(tone_message["embed"].title, "Alter (optional)")
            self.assertIsInstance(tone_message["view"], self.mod.OnboardingAgeStepView)

            age_view = self.mod.OnboardingAgeStepView(
                cog=cog, step_index=self.mod._AGE_STEP_INDEX, user_id=123456789
            )
            age_interaction = _FakeInteraction()
            age_skip = next(
                child for child in age_view.children if getattr(child, "label", "") == "Überspringen"
            )

            await age_skip.callback(age_interaction)

            self.assertEqual(tag_service.calls, [])
            age_message = age_interaction.response.messages[0]
            self.assertEqual(age_message["embed"].title, "🔗 Account verknüpfen & Rang-System")
            self.assertIsInstance(age_message["view"], self.mod.OnboardingAccountLinkView)
            self.assertEqual(pending_calls, [(123456789, 987654321)])

        asyncio.run(_run())


if __name__ == "__main__":
    unittest.main(verbosity=2)
