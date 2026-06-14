import asyncio
import json
import logging
import os
from typing import TYPE_CHECKING, Any
from collections.abc import Callable

from discord.ext import commands

try:
    import httpx
except Exception:  # pragma: no cover - optional dependency
    httpx = None  # type: ignore[assignment]

try:
    from openai import OpenAI
except Exception:  # pragma: no cover - optional dependency
    OpenAI = None  # type: ignore[assignment]

try:
    from google import genai
    from google.genai import types as genai_types
except Exception:  # pragma: no cover - optional dependency
    genai = None  # type: ignore[assignment]
    genai_types = None  # type: ignore[assignment]

log = logging.getLogger(__name__)

# --- MiniMax Defaults ---
DEFAULT_MINIMAX_MODEL = os.getenv("MINIMAX_MODEL", "MiniMax-M3")
DEFAULT_MINIMAX_BASE_URL = os.getenv("MINIMAX_BASE_URL", "https://api.minimax.chat/v1")
DEFAULT_MINIMAX_TOKEN_PLAN_BASE_URL = os.getenv(
    "MINIMAX_TOKEN_PLAN_BASE_URL",
    "https://api.minimax.io/anthropic/v1",
)

if TYPE_CHECKING:
    from openai import OpenAI as OpenAIClient
else:
    OpenAIClient = Any

DEFAULT_OPENAI_MODEL = os.getenv("AI_OPENAI_MODEL", "gpt-4o-mini")
DEFAULT_GEMINI_MODEL = os.getenv("AI_GEMINI_MODEL", "gemini-2.0-flash")
DEFAULT_MAX_OUTPUT_TOKENS = int(os.getenv("AI_MAX_OUTPUT_TOKENS", "800") or "800")


class AIConnector(commands.Cog):
    """Zentrale AI-Verbindungsstelle (Gemini & OpenAI) für alle Cogs."""

    def __init__(self, bot: commands.Bot):
        self.bot = bot
        self._openai_client: OpenAIClient | None = None
        self._openai_init_failed = False
        self._gemini_client: object | None = None
        self._gemini_init_failed = False

    # ---------- Clients ----------
    def _get_openai_client(self) -> OpenAIClient | None:
        if self._openai_init_failed:
            return None
        if self._openai_client:
            return self._openai_client

        api_key = os.getenv("OPENAI_API_KEY") or os.getenv("DEADLOCK_OPENAI_KEY")
        if not api_key:
            self._openai_init_failed = True
            return None
        if OpenAI is None:
            self._openai_init_failed = True
            log.debug("OpenAI SDK nicht verfügbar – OpenAI deaktiviert.")
            return None
        try:
            self._openai_client = OpenAI(api_key=api_key)
        except Exception as exc:
            self._openai_init_failed = True
            log.exception("OpenAI Client konnte nicht initialisiert werden: %s", exc)
            return None
        return self._openai_client

    def _get_gemini_client(self) -> object | None:
        if self._gemini_init_failed:
            return None
        if self._gemini_client:
            return self._gemini_client

        api_key = os.getenv("GOOGLE_API_KEY") or os.getenv("GEMINI_API_KEY")
        if not api_key:
            self._gemini_init_failed = True
            return None
        if genai is None or genai_types is None:
            self._gemini_init_failed = True
            log.debug("google-genai Paket fehlt – Gemini deaktiviert.")
            return None

        try:
            self._gemini_client = genai.Client(api_key=api_key)
        except Exception as exc:
            self._gemini_init_failed = True
            log.exception("Gemini Client konnte nicht initialisiert werden: %s", exc)
            return None
        return self._gemini_client

    def _get_minimax_client(self) -> tuple[Any, str, str] | None:
        """Returns (client, base_url, api_key) or None."""
        token_plan_key = os.getenv("MINIMAX_TOKEN_PLAN_KEY")
        api_key = token_plan_key or os.getenv("MINIMAX_API_KEY") or os.getenv("MINMAX")
        if not api_key:
            log.debug(
                "Kein MiniMax Key in ENV gefunden (geprueft: MINIMAX_TOKEN_PLAN_KEY, MINIMAX_API_KEY, MINMAX)"
            )
            return None
        if httpx is None:
            log.debug("httpx Paket fehlt – MiniMax deaktiviert.")
            return None
        use_token_plan = bool(token_plan_key and api_key == token_plan_key)
        base_url = (
            DEFAULT_MINIMAX_TOKEN_PLAN_BASE_URL if use_token_plan else DEFAULT_MINIMAX_BASE_URL
        )
        log.info(
            "MiniMax client init: mode=%s base_url=%s key_env=%s",
            "token_plan" if use_token_plan else "standard",
            base_url,
            "MINIMAX_TOKEN_PLAN_KEY"
            if use_token_plan
            else ("MINIMAX_API_KEY" if os.getenv("MINIMAX_API_KEY") else "MINMAX"),
        )

        class _MiniMaxClient:
            def __init__(self, api_key: str, base_url: str, use_token_plan: bool) -> None:
                self.api_key = api_key
                self.base_url = base_url
                self.use_token_plan = use_token_plan
                self._http = httpx.Client(timeout=60.0)

            @staticmethod
            def _parse_data_image_uri(image_url: str) -> tuple[str, str] | None:
                if not image_url.startswith("data:image/"):
                    return None
                header, separator, data = image_url.partition(",")
                if not separator or ";base64" not in header:
                    return None
                media_type = header[5:].split(";", 1)[0].strip() or "image/png"
                if not media_type.startswith("image/"):
                    return None
                return media_type, data

            def _build_token_plan_content(
                self, prompt: str, image_urls: list[str] | None
            ) -> list[dict[str, Any]]:
                content: list[dict[str, Any]] = [{"type": "text", "text": prompt}]
                for image_url in image_urls or []:
                    parsed_data_uri = self._parse_data_image_uri(image_url)
                    if parsed_data_uri:
                        media_type, data = parsed_data_uri
                        content.append(
                            {
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": media_type,
                                    "data": data,
                                },
                            }
                        )
                        continue

                    content.append(
                        {
                            "type": "image",
                            "source": {
                                "type": "url",
                                "url": image_url,
                            },
                        }
                    )
                return content

            def _build_standard_user_content(
                self, prompt: str, image_urls: list[str] | None
            ) -> str | list[dict[str, Any]]:
                if not image_urls:
                    return prompt

                content: list[dict[str, Any]] = [{"type": "text", "text": prompt}]
                for image_url in image_urls:
                    content.append(
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": image_url,
                            },
                        }
                    )
                return content

            def generate(
                self,
                *,
                prompt: str,
                system_prompt: str | None,
                model: str,
                max_output_tokens: int,
                temperature: float,
                image_urls: list[str] | None = None,
            ) -> str | None:
                if self.use_token_plan:
                    headers = {
                        "x-api-key": self.api_key,
                        "anthropic-version": "2023-06-01",
                        "Content-Type": "application/json",
                    }
                    payload = {
                        "model": model if model != "MiniMax-Text-01" else "MiniMax-M3",
                        "system": system_prompt or "",
                        "messages": [
                            {
                                "role": "user",
                                "content": self._build_token_plan_content(prompt, image_urls),
                            }
                        ],
                        "max_tokens": max_output_tokens,
                        "temperature": temperature,
                    }
                    resp = self._http.post(
                        f"{self.base_url}/messages",
                        headers=headers,
                        json=payload,
                    )
                else:
                    headers = {
                        "Authorization": f"Bearer {self.api_key}",
                        "Content-Type": "application/json",
                    }
                    messages = []
                    if system_prompt:
                        messages.append({"role": "system", "content": system_prompt})
                    messages.append(
                        {
                            "role": "user",
                            "content": self._build_standard_user_content(prompt, image_urls),
                        }
                    )

                    payload = {
                        "model": model,
                        "messages": messages,
                        "max_tokens": max_output_tokens,
                        "temperature": temperature,
                    }
                    resp = self._http.post(
                        f"{self.base_url}/text/chatcompletion_v2",
                        headers=headers,
                        json=payload,
                    )
                if resp.status_code != 200:
                    log.warning("MiniMax API Fehler: %s - %s", resp.status_code, resp.text)
                    return None
                data = resp.json()

                if self.use_token_plan:
                    fragments = []
                    for item in data.get("content", []) or []:
                        if item.get("type") == "text" and item.get("text"):
                            fragments.append(str(item["text"]))
                    if fragments:
                        return "".join(fragments).strip()
                    log.warning("MiniMax Token Plan Antwort ohne Textinhalt: %s", data)
                    return None

                choices = data.get("choices", [])
                if choices:
                    return choices[0].get("message", {}).get("content", "")
                return None

            def generate_with_tools(
                self,
                *,
                prompt: str,
                system_prompt: str | None,
                model: str,
                max_output_tokens: int,
                temperature: float,
                tools: list[dict[str, Any]],
                tool_executor: "Callable[[str, dict[str, Any]], Any]",
                max_tool_calls: int = 4,
            ) -> tuple[str | None, list[str]]:
                # Tool-Loop nur im token_plan-Modus (Anthropic-Messages-Format).
                if not self.use_token_plan:
                    return (None, [])

                headers = {
                    "x-api-key": self.api_key,
                    "anthropic-version": "2023-06-01",
                    "Content-Type": "application/json",
                }
                normalized_model = model if model != "MiniMax-Text-01" else "MiniMax-M3"
                messages: list[dict[str, Any]] = [
                    {"role": "user", "content": [{"type": "text", "text": prompt}]}
                ]
                used_tool_names: list[str] = []
                tool_calls_made = 0

                def _extract_text(content_blocks: list[dict[str, Any]]) -> str | None:
                    fragments = [
                        str(block.get("text", ""))
                        for block in content_blocks or []
                        if block.get("type") == "text" and block.get("text")
                    ]
                    joined = "".join(fragments).strip()
                    return joined or None

                try:
                    while True:
                        budget_left = tool_calls_made < max_tool_calls
                        payload: dict[str, Any] = {
                            "model": normalized_model,
                            "system": system_prompt or "",
                            "messages": messages,
                            "max_tokens": max_output_tokens,
                            "temperature": temperature,
                        }
                        # Solange Tool-Budget übrig ist, Tools anbieten.
                        if budget_left:
                            payload["tools"] = tools

                        resp = self._http.post(
                            f"{self.base_url}/messages",
                            headers=headers,
                            json=payload,
                        )
                        if resp.status_code != 200:
                            log.warning(
                                "MiniMax Tool-Loop API Fehler: %s - %s",
                                resp.status_code,
                                resp.text,
                            )
                            return (None, used_tool_names)

                        data = resp.json()
                        content_blocks = data.get("content", []) or []

                        if data.get("stop_reason") == "tool_use" and budget_left:
                            tool_result_blocks: list[dict[str, Any]] = []
                            for block in content_blocks:
                                if block.get("type") != "tool_use":
                                    continue
                                tool_name = block.get("name", "")
                                tool_input = block.get("input") or {}
                                try:
                                    result = tool_executor(tool_name, tool_input)
                                except Exception:
                                    log.exception(
                                        "MiniMax Tool-Executor fehlgeschlagen: %s", tool_name
                                    )
                                    result = {"error": "tool_execution_failed"}
                                used_tool_names.append(tool_name)
                                tool_result_blocks.append(
                                    {
                                        "type": "tool_result",
                                        "tool_use_id": block.get("id"),
                                        "content": json.dumps(result, ensure_ascii=False),
                                    }
                                )
                            # Defekte Antwort: stop_reason=tool_use, aber kein tool_use-Block.
                            # Keine leeren Turns anhängen, sondern vorhandenen Text zurückgeben.
                            if not tool_result_blocks:
                                return (_extract_text(content_blocks), used_tool_names)
                            # Assistant-Turn (Original-Content) + User-Turn mit tool_results anhängen.
                            messages.append({"role": "assistant", "content": content_blocks})
                            messages.append({"role": "user", "content": tool_result_blocks})
                            tool_calls_made += 1
                            continue

                        # Keine weiteren Tool-Calls (oder Budget erschöpft) -> Textantwort.
                        return (_extract_text(content_blocks), used_tool_names)
                except Exception:
                    log.exception("MiniMax Tool-Loop fehlgeschlagen")
                    return (None, used_tool_names)

        return (_MiniMaxClient(api_key, base_url, use_token_plan), base_url, api_key)

    # ---------- Public API ----------
    async def generate_text(
        self,
        *,
        provider: str,
        prompt: str,
        system_prompt: str | None = None,
        model: str | None = None,
        max_output_tokens: int | None = None,
        temperature: float = 0.6,
    ) -> tuple[str | None, dict[str, Any]]:
        """
        Einfache Text-Generierung über Gemini oder OpenAI Responses API.
        Returns (text|None, meta)
        """
        provider = provider.lower()
        meta: dict[str, Any] = {
            "provider": provider,
            "model": model,
        }
        mot = max_output_tokens or DEFAULT_MAX_OUTPUT_TOKENS

        if provider == "gemini":
            text = await self._generate_gemini(
                prompt=prompt,
                system_prompt=system_prompt,
                model=model or DEFAULT_GEMINI_MODEL,
                max_output_tokens=mot,
                temperature=temperature,
            )
            meta["model"] = model or DEFAULT_GEMINI_MODEL
            if text is None:
                meta["error"] = "gemini_unavailable"
            return text, meta

        if provider == "openai":
            text, usage = await self._generate_openai(
                prompt=prompt,
                system_prompt=system_prompt,
                model=model or DEFAULT_OPENAI_MODEL,
                max_output_tokens=mot,
                temperature=temperature,
            )
            meta["model"] = model or DEFAULT_OPENAI_MODEL
            if usage:
                meta["usage"] = usage
            if text is None:
                meta["error"] = "openai_unavailable"
            return text, meta

        if provider == "minimax":
            text = await self._generate_minimax(
                prompt=prompt,
                system_prompt=system_prompt,
                model=model or DEFAULT_MINIMAX_MODEL,
                max_output_tokens=mot,
                temperature=temperature,
            )
            meta["model"] = model or DEFAULT_MINIMAX_MODEL
            if text is None:
                meta["error"] = "minimax_unavailable"
            return text, meta

        meta["error"] = "unknown_provider"
        return None, meta

    async def generate_text_with_tools(
        self,
        *,
        provider: str,
        prompt: str,
        system_prompt: str | None = None,
        model: str | None = None,
        max_output_tokens: int | None = None,
        temperature: float = 0.6,
        tools: list[dict[str, Any]],
        tool_executor: "Callable[[str, dict[str, Any]], Any]",
        max_tool_calls: int = 4,
    ) -> tuple[str, dict[str, Any]]:
        """
        Anthropic-kompatibler Tool-Use-Loop über MiniMax (nur token_plan-Modus).
        tool_executor ist eine SYNC-Callable und läuft im Worker-Thread.
        Returns (text, meta).
        """
        if provider.lower() != "minimax":
            return ("", {"error": "unsupported_provider"})

        client_data = self._get_minimax_client()
        if not client_data:
            return ("", {"error": "minimax_unavailable"})
        client, _, _ = client_data

        resolved_model = model or DEFAULT_MINIMAX_MODEL
        mot = max_output_tokens or DEFAULT_MAX_OUTPUT_TOKENS

        def _call_model() -> tuple[str | None, list[str]]:
            return client.generate_with_tools(
                prompt=prompt,
                system_prompt=system_prompt,
                model=resolved_model,
                max_output_tokens=mot,
                temperature=temperature,
                tools=tools,
                tool_executor=tool_executor,
                max_tool_calls=max_tool_calls,
            )

        text, used_tool_names = await asyncio.to_thread(_call_model)
        return (
            text or "",
            {
                "provider": "minimax",
                "model": resolved_model,
                "tool_calls": used_tool_names,
            },
        )

    async def generate_multimodal(
        self,
        *,
        provider: str,
        prompt: str,
        images: list[str],
        system_prompt: str | None = None,
        model: str | None = None,
        max_output_tokens: int | None = None,
        temperature: float = 0.2,
    ) -> tuple[str | None, dict[str, Any]]:
        provider = provider.lower()
        meta: dict[str, Any] = {
            "provider": provider,
            "model": model,
        }

        if provider != "minimax":
            meta["error"] = "multimodal_not_supported"
            return None, meta

        valid_prefixes = ("http://", "https://", "data:image/")
        valid_images = [image for image in images if image.startswith(valid_prefixes)]
        invalid_count = len(images) - len(valid_images)
        if invalid_count:
            log.debug("MiniMax multimodal: %s ungueltige Bild-URLs uebersprungen.", invalid_count)

        dropped_images = valid_images[4:]
        if dropped_images:
            meta["dropped_images"] = dropped_images
        selected_images = valid_images[:4]

        mot = max_output_tokens or DEFAULT_MAX_OUTPUT_TOKENS
        resolved_model = model or DEFAULT_MINIMAX_MODEL
        text = await self._generate_minimax_multimodal(
            prompt=prompt,
            images=selected_images,
            system_prompt=system_prompt,
            model=resolved_model,
            max_output_tokens=mot,
            temperature=temperature,
        )
        meta["model"] = resolved_model
        if text is None:
            meta["error"] = "minimax_unavailable"
        return text, meta

    # ---------- Provider Implementierungen ----------
    async def _generate_gemini(
        self,
        *,
        prompt: str,
        system_prompt: str | None,
        model: str,
        max_output_tokens: int,
        temperature: float,
    ) -> str | None:
        client = self._get_gemini_client()
        if not client or genai_types is None:
            return None

        contents = prompt
        if system_prompt:
            contents = f"{system_prompt}\n\n{prompt}"

        def _call_model() -> str | None:
            try:
                response = client.models.generate_content(
                    model=model,
                    contents=contents,
                    config=genai_types.GenerateContentConfig(
                        temperature=temperature,
                        max_output_tokens=max_output_tokens,
                    ),
                )
                text = getattr(response, "text", "") or ""
                return text.strip() if text else None
            except Exception as exc:
                log.debug("Gemini Request fehlgeschlagen: %s", exc)
                return None

        return await asyncio.to_thread(_call_model)

    async def _generate_openai(
        self,
        *,
        prompt: str,
        system_prompt: str | None,
        model: str,
        max_output_tokens: int,
        temperature: float,
    ) -> tuple[str | None, dict[str, Any] | None]:
        client = self._get_openai_client()
        if not client:
            return None, None

        def _call_model():
            try:
                return client.responses.create(
                    model=model,
                    input=prompt,
                    instructions=system_prompt,
                    max_output_tokens=max_output_tokens,
                    temperature=temperature,
                )
            except TypeError:
                return client.responses.create(
                    model=model,
                    input=prompt,
                    instructions=system_prompt,
                    max_tokens=max_output_tokens,
                    temperature=temperature,
                )
            except Exception as exc:
                log.debug("OpenAI Request fehlgeschlagen: %s", exc)
                return None

        response = await asyncio.to_thread(_call_model)
        if response is None:
            return None, None

        # Extract text
        text = ""
        try:
            output_text = getattr(response, "output_text", None)
            if not output_text and isinstance(response, dict):
                output_text = response.get("output_text")
            if output_text:
                text = str(output_text).strip()
            else:
                out = getattr(response, "output", None) or getattr(response, "outputs", None)
                if out is None and isinstance(response, dict):
                    out = response.get("output") or response.get("outputs")
                fragments = []
                for item in out or []:
                    item_type = getattr(item, "type", None)
                    if item_type is None and isinstance(item, dict):
                        item_type = item.get("type")
                    if item_type != "message":
                        continue
                    item_content = getattr(item, "content", None)
                    if item_content is None and isinstance(item, dict):
                        item_content = item.get("content")
                    for part in item_content or []:
                        txt = getattr(part, "text", None)
                        if txt is None and isinstance(part, dict):
                            txt = part.get("text")
                        if txt:
                            fragments.append(str(txt))
                text = "".join(fragments).strip()
        except Exception:
            log.exception("Antwort-Parsing fehlgeschlagen")
            text = ""

        usage_raw = getattr(response, "usage", None)
        if usage_raw is None and isinstance(response, dict):
            usage_raw = response.get("usage")
        usage = None
        if usage_raw:
            usage = {
                "input_tokens": getattr(usage_raw, "input_tokens", None),
                "output_tokens": getattr(usage_raw, "output_tokens", None),
                "total_tokens": getattr(usage_raw, "total_tokens", None),
            }
        return text or None, usage

    async def _generate_minimax(
        self,
        *,
        prompt: str,
        system_prompt: str | None,
        model: str,
        max_output_tokens: int,
        temperature: float,
    ) -> str | None:
        client_data = self._get_minimax_client()
        if not client_data:
            return None
        client, _, _ = client_data

        def _call_model() -> str | None:
            return client.generate(
                prompt=prompt,
                system_prompt=system_prompt,
                model=model,
                max_output_tokens=max_output_tokens,
                temperature=temperature,
            )

        return await asyncio.to_thread(_call_model)

    async def _generate_minimax_multimodal(
        self,
        *,
        prompt: str,
        images: list[str],
        system_prompt: str | None,
        model: str,
        max_output_tokens: int,
        temperature: float,
    ) -> str | None:
        client_data = self._get_minimax_client()
        if not client_data:
            return None
        client, _, _ = client_data

        def _call_model() -> str | None:
            return client.generate(
                prompt=prompt,
                system_prompt=system_prompt,
                model=model,
                max_output_tokens=max_output_tokens,
                temperature=temperature,
                image_urls=images,
            )

        return await asyncio.to_thread(_call_model)

    # ---------- Commands ----------
    @commands.command(name="aiob")
    @commands.has_permissions(administrator=True)
    async def ai_onboarding_test(self, ctx: commands.Context):
        """
        Schickt dir den AI-Onboarding Start-Button in die DMs.
        Nutzt AIOnboarding Cog, falls geladen.
        """
        ai_ob = getattr(self.bot, "get_cog", lambda name: None)("AIOnboarding")
        if not ai_ob or not hasattr(ai_ob, "start_in_channel"):
            await ctx.reply(
                "AIOnboarding ist nicht geladen. Bitte Cog laden und erneut versuchen.",
                mention_author=False,
            )
            return

        try:
            dm = ctx.author.dm_channel or await ctx.author.create_dm()
        except Exception as exc:
            log.warning("Konnte DM fuer aiob nicht oeffnen: %s", exc)
            await ctx.reply("Konnte deine DMs nicht oeffnen.", mention_author=False)
            return

        ok = await ai_ob.start_in_channel(dm, ctx.author)  # type: ignore[attr-defined]
        if ok:
            await ctx.reply(
                "AI-Onboarding Test wurde an deine DMs geschickt. Keine automatischen DMs aktiv.",
                mention_author=False,
            )
        else:
            await ctx.reply("Konnte den AI-Onboarding Test nicht starten.", mention_author=False)


async def setup(bot: commands.Bot):
    await bot.add_cog(AIConnector(bot))
