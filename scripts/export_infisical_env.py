#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
from urllib import error, parse, request


def _required(name: str) -> str:
    value = (os.getenv(name) or "").strip()
    if not value:
        raise SystemExit(f"Missing required environment variable: {name}")
    return value


def _fetch_secrets() -> list[dict[str, object]]:
    base_url = _required("INFISICAL_API_URL")
    parsed_url = parse.urlsplit(base_url)
    if parsed_url.scheme not in {"http", "https"} or not parsed_url.netloc:
        raise SystemExit("INFISICAL_API_URL must use http/https")
    base_url = base_url.rstrip("/")
    project_id = _required("INFISICAL_PROJECT_ID")
    environment = _required("INFISICAL_ENV")
    service_token = _required("INFISICAL_SERVICE_TOKEN")
    secret_path = (os.getenv("INFISICAL_SECRET_PATH") or "/").strip() or "/"
    timeout = float(os.getenv("INFISICAL_HTTP_TIMEOUT", "10"))

    query = parse.urlencode(
        {
            "projectId": project_id,
            "environment": environment,
            "secretPath": secret_path,
            "viewSecretValue": "true",
            "includeImports": "true",
            "recursive": "false",
        }
    )
    req = request.Request(  # noqa: S310 - scheme is restricted above
        f"{base_url}/api/v4/secrets/?{query}",
        headers={
            "Authorization": f"Bearer {service_token}",
            "Accept": "application/json",
            "User-Agent": "deadlock-bots-infisical-loader/1.0",
        },
        method="GET",
    )

    try:
        with request.urlopen(req, timeout=timeout) as resp:  # noqa: S310
            payload = json.loads(resp.read().decode("utf-8"))
    except error.HTTPError as exc:
        raise SystemExit(f"Infisical request failed with HTTP {exc.code}") from exc
    except (error.URLError, ConnectionResetError, TimeoutError, OSError) as exc:
        raise SystemExit(f"Infisical request failed: {exc}") from exc

    secrets = list(payload.get("secrets") or [])
    for imported in payload.get("imports") or []:
        secrets.extend(imported.get("secrets") or [])
    return secrets


def _as_env_map(items: list[dict[str, object]]) -> dict[str, str]:
    env_map: dict[str, str] = {}
    for item in items:
        key = str(item.get("secretKey") or "").strip()
        if not key:
            continue
        value = item.get("secretValue")
        env_map[key] = "" if value is None else str(value)

    if env_map.get("MINIMAX_TOKEN_PLAN_KEY") and not env_map.get("MINIMAX_API_KEY"):
        env_map["MINIMAX_API_KEY"] = env_map["MINIMAX_TOKEN_PLAN_KEY"]

    return env_map


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run a command with Deadlock-Bots secrets")
    parser.add_argument("--exec", dest="command", nargs=argparse.REMAINDER, required=True)
    args = parser.parse_args(argv)

    env_map = _as_env_map(_fetch_secrets())
    if not args.command:
        parser.error("--exec requires a command")
    environment = os.environ.copy()
    environment.update(env_map)
    environment.pop("INFISICAL_SERVICE_TOKEN", None)
    os.execvpe(args.command[0], args.command, environment)  # noqa: S606
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
