#!/usr/bin/env python3
"""Claude-stdio-Brücke zum lokalen dl-bot-MCP mit Infisical-Bearer-Auth."""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path
from urllib import error, request

import tomllib

SECRET_NAME = "TWITCH_INTERNAL_API_TOKEN"
LOADER = Path("/home/nathanael/Documents/Infisical/export_gpt_secret.py")
CONFIG = Path("/home/nathanael/.config/deadlock-bots/bot.toml")
MAX_MESSAGE_BYTES = 2 * 1024 * 1024


class NoRedirect(request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def load_token(loader: Path = LOADER) -> str:
    """Lädt genau den bestehenden internen Token; Fehler enthalten keinen Wert."""
    try:
        spec = importlib.util.spec_from_file_location("dl_bot_mcp_infisical", loader)
        if spec is None or spec.loader is None:
            raise RuntimeError("Loader fehlt")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        token = str(module._fetch_secret(SECRET_NAME)).strip()
    except (Exception, SystemExit):  # noqa: BLE001 - fremde Loader-Fehler nie mit Secret ausgeben
        raise RuntimeError(
            "MCP-Token konnte nicht aus Infisical geladen werden"
        ) from None
    if not token:
        raise RuntimeError("MCP-Token fehlt in Infisical")
    return token


def endpoint_from_config(path: Path = CONFIG) -> str:
    """Liest denselben nichtgeheimen MCP-Port wie der dl-bot aus normaler TOML."""
    try:
        raw = tomllib.loads(path.read_text(encoding="utf-8"))
        port = raw.get("runtime", {}).get("start", {}).get("mcp_port", 8890)
    except (OSError, ValueError, AttributeError, TypeError):
        raise RuntimeError("MCP-Betriebsconfig ist nicht lesbar") from None
    if isinstance(port, bool) or not isinstance(port, int) or not 1 <= port <= 65535:
        raise RuntimeError("MCP-Port in TOML ist ungültig")
    return f"http://127.0.0.1:{port}/mcp"


def rpc_ids(payload: bytes) -> tuple[list[object], bool]:
    try:
        value = json.loads(payload)
    except (ValueError, UnicodeDecodeError):
        return [], False
    if isinstance(value, list):
        return [
            item["id"]
            for item in value
            if isinstance(item, dict) and item.get("id") is not None
        ], True
    if isinstance(value, dict) and value.get("id") is not None:
        return [value["id"]], False
    return [], False


def error_response(payload: bytes) -> bytes | None:
    ids, batch = rpc_ids(payload)
    if not ids:
        return None
    errors = [
        {
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {"code": -32000, "message": "Discord-MCP ist nicht erreichbar"},
        }
        for request_id in ids
    ]
    return json.dumps(
        errors if batch else errors[0],
        ensure_ascii=False,
        separators=(",", ":"),
    ).encode("utf-8")


def forward(
    payload: bytes, token: str, opener: request.OpenerDirector, endpoint: str
) -> bytes | None:
    message = request.Request(
        endpoint,
        data=payload,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "Accept": "application/json",
        },
        method="POST",
    )
    try:
        with opener.open(message, timeout=180) as response:
            if response.status == 202:
                return None
            if response.status != 200:
                raise RuntimeError("MCP-Status")
            body = response.read(MAX_MESSAGE_BYTES + 1)
    except (error.HTTPError, error.URLError, TimeoutError, OSError, RuntimeError):
        return error_response(payload)
    if len(body) > MAX_MESSAGE_BYTES:
        return error_response(payload)
    try:
        json.loads(body)
    except (ValueError, UnicodeDecodeError):
        return error_response(payload)
    return body


def main() -> int:
    try:
        token = load_token()
    except RuntimeError as exc:
        print(exc, file=sys.stderr)
        return 1
    opener = request.build_opener(request.ProxyHandler({}), NoRedirect())
    while line := sys.stdin.buffer.readline(MAX_MESSAGE_BYTES + 1):
        if len(line) > MAX_MESSAGE_BYTES or not line.endswith(b"\n"):
            print("MCP-Nachricht ist zu groß oder unvollständig", file=sys.stderr)
            return 1
        try:
            endpoint = endpoint_from_config()
        except RuntimeError:
            result = error_response(line)
        else:
            result = forward(line, token, opener, endpoint)
        if result is not None:
            sys.stdout.buffer.write(result.rstrip(b"\n") + b"\n")
            sys.stdout.buffer.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
