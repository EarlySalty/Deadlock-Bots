from __future__ import annotations

import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def test_infisical_exporter_exists_and_validates_required_env() -> None:
    script = ROOT / "scripts" / "export_infisical_env.py"

    assert script.is_file()
    result = subprocess.run(
        [sys.executable, str(script), "--format", "shell"],
        capture_output=True,
        env={},
        text=True,
    )

    assert result.returncode != 0
    assert result.stderr.strip() == "Missing required environment variable: INFISICAL_API_URL"
