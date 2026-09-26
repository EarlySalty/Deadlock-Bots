#!/usr/bin/env python3
"""Claude-stdio-Brücke zum lokalen dl-bot-MCP mit Infisical-Bearer-Auth."""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path
from urllib import error, request

ENDPOINT = "http://127.0.0.1:8890/mcp"
SECRET_NAME = "TWITCH_INTERNAL_API_TOKEN"
LOADER = Path("/home/nathanael/Documents/Infisical/export_gpt_secret.py")
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


def rpc_id(payload: bytes) -> object | None:
    try:
        value = json.loads(payload)
    except (ValueError, UnicodeDecodeError):
        return None
    return value.get("id") if isinstance(value, dict) else None


def error_response(request_id: object) -> bytes:
    return json.dumps(
        {
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {"code": -32000, "message": "Discord-MCP ist nicht erreichbar"},
        },
        ensure_ascii=False,
        separators=(",", ":"),
    ).encode("utf-8")


def forward(payload: bytes, token: str, opener: request.OpenerDirector) -> bytes | None:
    message = request.Request(
        ENDPOINT,
        data=payload,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "Accept": "application/json",
        },
        method="POST",
    )
    try:
        with opener.open(message, timeout=30) as response:
            if response.status == 202:
                return None
            if response.status != 200:
                raise RuntimeError("MCP-Status")
            body = response.read(MAX_MESSAGE_BYTES + 1)
    except (error.HTTPError, error.URLError, TimeoutError, OSError, RuntimeError):
        request_id = rpc_id(payload)
        return error_response(request_id) if request_id is not None else None
    if len(body) > MAX_MESSAGE_BYTES:
        request_id = rpc_id(payload)
        return error_response(request_id) if request_id is not None else None
    try:
        json.loads(body)
    except (ValueError, UnicodeDecodeError):
        request_id = rpc_id(payload)
        return error_response(request_id) if request_id is not None else None
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
        result = forward(line, token, opener)
        if result is not None:
            sys.stdout.buffer.write(result.rstrip(b"\n") + b"\n")
            sys.stdout.buffer.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
