#!/usr/bin/env python3
from contextlib import redirect_stderr, redirect_stdout
from importlib.util import module_from_spec, spec_from_file_location
from io import StringIO
from pathlib import Path
from subprocess import run
from unittest import TestCase, main, mock

SCRIPTS_DIR = Path(__file__).resolve().parent
spec = spec_from_file_location("export_infisical_env", SCRIPTS_DIR / "export_infisical_env.py")
assert spec is not None and spec.loader is not None
loader = module_from_spec(spec)
spec.loader.exec_module(loader)


class ExecCalled(Exception):
    pass


class KnowledgeLauncherTest(TestCase):
    def test_launcher_artefakte_sind_nicht_gitignoriert(self) -> None:
        for path in (
            "scripts/export_infisical_env.py",
            "scripts/test_dl_knowledge_launcher.py",
            "scripts/wait_for_infisical.sh",
        ):
            result = run(  # noqa: S603 - fixed git diagnostic, no shell
                ["git", "check-ignore", "--no-index", "-q", path],  # noqa: S607
                cwd=SCRIPTS_DIR.parent,
                check=False,
            )
            self.assertEqual(result.returncode, 1, path)

    def test_loader_akzeptiert_nur_exakten_lokalen_infisical_endpunkt(self) -> None:
        invalid = (
            "file:///tmp/not-infisical",
            "https://127.0.0.1:8080",
            "http://localhost:8080",
            "http://127.0.0.1:8081",
            "http://user@127.0.0.1:8080",
            "http://127.0.0.1:8080/anderer/pfad",
            "http://127.0.0.1:8080?redirect=evil",
        )
        for value in invalid:
            with (
                self.subTest(value=value),
                mock.patch.dict(loader.os.environ, {"INFISICAL_API_URL": value}, clear=True),
                self.assertRaisesRegex(SystemExit, "127.0.0.1:8080"),
            ):
                loader._validated_infisical_base_url()

        with mock.patch.dict(
            loader.os.environ,
            {"INFISICAL_API_URL": "http://127.0.0.1:8080/"},
            clear=True,
        ):
            self.assertEqual(loader._validated_infisical_base_url(), "http://127.0.0.1:8080")

    def test_loader_deaktiviert_proxy_und_redirects(self) -> None:
        with mock.patch.object(loader.request, "build_opener") as build_opener:
            loader._local_opener()

        handlers = build_opener.call_args.args
        proxy = next(
            handler for handler in handlers if isinstance(handler, loader.request.ProxyHandler)
        )
        redirect = next(handler for handler in handlers if isinstance(handler, loader._NoRedirect))
        self.assertEqual(proxy.proxies, {})
        self.assertIsNone(
            redirect.redirect_request(
                None, None, 302, "Found", {}, "http://127.0.0.1:8080/elsewhere"
            )
        )

    def test_loader_exec_injiziert_secrets_ohne_ausgabe(self) -> None:
        stdout = StringIO()
        with (
            mock.patch.object(
                loader,
                "_fetch_secrets",
                return_value=[
                    {"secretKey": "FIREWORK_API_KEY", "secretValue": "allowed-secret"},
                    {"secretKey": "TEST_SECRET", "secretValue": "must-not-reach-child"},
                ],
            ),
            mock.patch.object(loader.os, "execvpe", side_effect=ExecCalled) as execvpe,
            mock.patch.dict(
                loader.os.environ,
                {
                    "UNCHANGED": "yes",
                    "TEST_SECRET": "old-value",
                    "INFISICAL_SERVICE_TOKEN": "bootstrap-token",
                },
                clear=True,
            ),
            redirect_stdout(stdout),
            self.assertRaises(ExecCalled),
        ):
            loader.main(["--exec", "/bin/dl-knowledge", "--probe"])

        command, arguments, environment = execvpe.call_args.args
        self.assertEqual(command, "/bin/dl-knowledge")
        self.assertEqual(arguments, ["/bin/dl-knowledge", "--probe"])
        self.assertEqual(environment["FIREWORK_API_KEY"], "allowed-secret")
        self.assertNotIn("TEST_SECRET", environment)
        self.assertEqual(environment["UNCHANGED"], "yes")
        self.assertNotIn("INFISICAL_SERVICE_TOKEN", environment)
        self.assertEqual(stdout.getvalue(), "")

    def test_loader_bietet_keinen_secret_exportmodus_an(self) -> None:
        with redirect_stderr(StringIO()), self.assertRaises(SystemExit):
            loader.main([])

    def test_launcher_uebergibt_secrets_ohne_shell_ausgabe(self) -> None:
        wrapper = (SCRIPTS_DIR / "run_dl_knowledge_service.sh").read_text()

        self.assertIn("--exec", wrapper)
        self.assertNotIn("eval ", wrapper)
        self.assertNotIn("--format shell", wrapper)


if __name__ == "__main__":
    main()
