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

    def test_loader_lehnt_nicht_http_infisical_url_ab(self) -> None:
        environment = {
            "INFISICAL_API_URL": "file:///tmp/not-infisical",
            "INFISICAL_PROJECT_ID": "project",
            "INFISICAL_ENV": "prod",
            "INFISICAL_SERVICE_TOKEN": "bootstrap-token",
        }
        with (
            mock.patch.dict(loader.os.environ, environment, clear=True),
            mock.patch.object(loader.request, "urlopen") as urlopen,
            self.assertRaisesRegex(SystemExit, "http/https"),
        ):
            loader._fetch_secrets()

        urlopen.assert_not_called()

    def test_loader_exec_injiziert_secrets_ohne_ausgabe(self) -> None:
        stdout = StringIO()
        with (
            mock.patch.object(
                loader,
                "_fetch_secrets",
                return_value=[{"secretKey": "TEST_SECRET", "secretValue": "top-secret"}],
            ),
            mock.patch.object(loader.os, "execvpe", side_effect=ExecCalled) as execvpe,
            mock.patch.dict(
                loader.os.environ,
                {"UNCHANGED": "yes", "INFISICAL_SERVICE_TOKEN": "bootstrap-token"},
                clear=True,
            ),
            redirect_stdout(stdout),
            self.assertRaises(ExecCalled),
        ):
            loader.main(["--exec", "/bin/dl-knowledge", "--probe"])

        command, arguments, environment = execvpe.call_args.args
        self.assertEqual(command, "/bin/dl-knowledge")
        self.assertEqual(arguments, ["/bin/dl-knowledge", "--probe"])
        self.assertEqual(environment["TEST_SECRET"], "top-secret")
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
