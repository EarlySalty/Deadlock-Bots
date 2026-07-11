#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
from urllib import error, parse, request

INFISICAL_BASE_URL = "http://127.0.0.1:8080"
KNOWLEDGE_ENV_ALLOWLIST = frozenset(
    {
        "DL_LLM_MODEL_BOT_PATE",
        "FIREWORK_API_KEY",
        "FIREWORK_BASE_URL",
        "FIREWORK_MODEL",
        "FIREWORKS_API_KEY",
        "FIREWORKS_BASE_URL",
        "FIREWORKS_MODEL",
    }
)


class _NoRedirect(request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: ANN001
        return None


def _required(name: str) -> str:
    value = (os.getenv(name) or "").strip()
    if not value:
        raise SystemExit(f"Missing required environment variable: {name}")
    return value


def _validated_infisical_base_url() -> str:
    base_url = _required("INFISICAL_API_URL")
    parsed = parse.urlsplit(base_url)
    if (
        parsed.scheme != "http"
        or parsed.netloc != "127.0.0.1:8080"
        or parsed.path not in {"", "/"}
        or parsed.query
        or parsed.fragment
        or parsed.username is not None
        or parsed.password is not None
    ):
        raise SystemExit(f"INFISICAL_API_URL must be exactly {INFISICAL_BASE_URL}")
    return INFISICAL_BASE_URL


def _local_opener():
    return request.build_opener(request.ProxyHandler({}), _NoRedirect())


def _fetch_secrets() -> list[dict[str, object]]:
    base_url = _validated_infisical_base_url()
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
        with _local_opener().open(req, timeout=timeout) as resp:  # noqa: S310
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

    return env_map


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run a command with Deadlock-Bots secrets")
    parser.add_argument("--exec", dest="command", nargs=argparse.REMAINDER, required=True)
    args = parser.parse_args(argv)

    fetched_env = _as_env_map(_fetch_secrets())
    env_map = {key: value for key, value in fetched_env.items() if key in KNOWLEDGE_ENV_ALLOWLIST}
    if not args.command:
        parser.error("--exec requires a command")
    environment = os.environ.copy()
    for key in fetched_env:
        environment.pop(key, None)
    environment.update(env_map)
    environment.pop("INFISICAL_SERVICE_TOKEN", None)
    os.execvpe(args.command[0], args.command, environment)  # noqa: S606
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
