"""Vertrag der lokalen Claude-MCP-Brücke ohne echte Secrets oder Netzwerk."""

import importlib.util
from pathlib import Path
from urllib import error

SCRIPT = Path(__file__).with_name("dl_bot_mcp_stdio_proxy.py")
spec = importlib.util.spec_from_file_location("dl_bot_mcp_stdio_proxy", SCRIPT)
assert spec is not None and spec.loader is not None
proxy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proxy)


class FakeResponse:
    def __init__(self, status: int, body: bytes):
        self.status = status
        self.body = body

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False

    def read(self, _size):
        return self.body


class FakeOpener:
    def __init__(self, response):
        self.response = response
        self.last_request = None

    def open(self, req, timeout):
        assert timeout == 180
        self.last_request = req
        if isinstance(self.response, Exception):
            raise self.response
        return self.response


def test_proxy_liest_nur_bestehenden_infisical_token(tmp_path):
    loader = tmp_path / "fake_loader.py"
    loader.write_text(
        "def _fetch_secret(name):\n"
        "    assert name == 'TWITCH_INTERNAL_API_TOKEN'\n"
        "    return ' test-token '\n",
        encoding="utf-8",
    )
    assert proxy.load_token(loader) == "test-token"


def test_proxy_sendet_bearer_nur_an_festen_loopback_endpoint():
    opener = FakeOpener(FakeResponse(200, b'{"jsonrpc":"2.0","id":1,"result":{}}'))
    result = proxy.forward(
        b'{"jsonrpc":"2.0","id":1,"method":"ping"}\n',
        "test-token",
        opener,
        "http://127.0.0.1:8890/mcp",
    )
    assert result == b'{"jsonrpc":"2.0","id":1,"result":{}}'
    assert opener.last_request.full_url == "http://127.0.0.1:8890/mcp"
    assert opener.last_request.get_header("Authorization") == "Bearer test-token"
    assert opener.last_request.data.endswith(b"\n")


def test_redirect_darf_bearer_nicht_an_fremdes_ziel_tragen():
    assert (
        proxy.NoRedirect().redirect_request(
            None, None, 302, "Found", {}, "http://example.invalid/collect"
        )
        is None
    )


def test_proxy_verwirft_notification_und_meldet_http_fehler_ohne_token():
    opener = FakeOpener(error.URLError("Netz nicht erreichbar"))
    notification = b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n'
    endpoint = "http://127.0.0.1:8890/mcp"
    assert proxy.forward(notification, "test-token", opener, endpoint) is None
    result = proxy.forward(
        b'{"jsonrpc":"2.0","id":7,"method":"ping"}\n',
        "test-token",
        opener,
        endpoint,
    )
    assert result is not None and b'"id":7' in result
    assert b"test-token" not in result


def test_proxy_folgt_dem_toml_port_und_lehnt_ungueltige_werte_ab(tmp_path):
    config = tmp_path / "bot.toml"
    config.write_text("[runtime.start]\nmcp_port = 8891\n", encoding="utf-8")
    assert proxy.endpoint_from_config(config) == "http://127.0.0.1:8891/mcp"
    config.write_text("[runtime.start]\nmcp_port = 8892\n", encoding="utf-8")
    assert proxy.endpoint_from_config(config) == "http://127.0.0.1:8892/mcp"
    config.write_text("[runtime.start]\nmcp_port = true\n", encoding="utf-8")
    try:
        proxy.endpoint_from_config(config)
    except RuntimeError:
        pass
    else:
        raise AssertionError("Bool-Port wurde akzeptiert")


def test_batch_fehlermeldung_antwortet_auf_jede_id():
    payload = (
        b'[{"jsonrpc":"2.0","id":1,"method":"ping"},'
        b'{"jsonrpc":"2.0","method":"notifications/initialized"},'
        b'{"jsonrpc":"2.0","id":2,"method":"tools/list"}]\n'
    )
    result = proxy.forward(
        payload,
        "test-token",
        FakeOpener(error.URLError("Netz nicht erreichbar")),
        "http://127.0.0.1:8890/mcp",
    )
    assert result is not None
    assert proxy.json.loads(result) == [
        {
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {"code": -32000, "message": "Discord-MCP ist nicht erreichbar"},
        }
        for request_id in (1, 2)
    ]


def test_proxy_verwirft_leeren_oder_fehlenden_secretwert(tmp_path):
    loader = tmp_path / "fake_loader.py"
    loader.write_text("def _fetch_secret(_name): return ''\n", encoding="utf-8")
    try:
        proxy.load_token(loader)
    except RuntimeError as exc:
        assert "Infisical" in str(exc)
    else:
        raise AssertionError("Leeres Secret wurde akzeptiert")
