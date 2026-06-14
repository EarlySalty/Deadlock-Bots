"""Guard-Layer fuer Diagnose-Antworten des FAQ/Ticket-Bots.

Zweite, unabhaengige Output-Pruefung VOR dem Posten einer Kandidatenantwort.
Zweistufig:
  1. Deterministischer Scan (hartes Stop) auf Secret-Muster und fremde
     Discord-IDs.
  2. MiniMax-Reviewer als zweite Meinung.

Fail-closed: Bei jedem Fehler/Zweifel wird NICHT freigegeben (lieber
schweigen als geheime/fremde Daten leaken).
"""

from __future__ import annotations

import logging
import os
import re
from typing import Any

log = logging.getLogger(__name__)

# --- Deterministische Muster (hartes Stop) ---
# Secret-/Credential-Indikatoren. Bewusst grob: lieber einmal zu viel
# blockieren als ein Token durchlassen.
_SECRET_PATTERNS: tuple[re.Pattern[str], ...] = (
    re.compile(r"postgres://", re.IGNORECASE),
    re.compile(r"redis://", re.IGNORECASE),
    re.compile(r"\bBearer\b", re.IGNORECASE),
    re.compile(r"token\s*=", re.IGNORECASE),
    re.compile(r"key\s*=", re.IGNORECASE),
    re.compile(r"x-api-key", re.IGNORECASE),
    re.compile(r"INFISICAL", re.IGNORECASE),
    re.compile(r"oauth:[A-Za-z0-9]+", re.IGNORECASE),
    # Token-/Hash-artige Strings (>= 24 Zeichen). WICHTIG: nur wenn Ziffern UND
    # Buchstaben gemischt sind — sonst wuerden lange deutsche Woerter (z. B.
    # "Benachrichtigungseinstellungen") faelschlich als Secret gewertet und der
    # fail-closed-Guard wuerde legitime Antworten stumm blockieren.
    re.compile(
        r"\b(?=[A-Za-z0-9+/_-]*[0-9])(?=[A-Za-z0-9+/_-]*[A-Za-z])[A-Za-z0-9+/_-]{24,}\b"
    ),
    # Reine Hex-Strings (>= 24) und base64 mit Padding.
    re.compile(r"\b[0-9a-fA-F]{24,}\b"),
    re.compile(r"\b[A-Za-z0-9+/]{24,}={1,2}"),
)

# 17-20-stellige Zahlen -> Discord-Snowflake-IDs.
_DISCORD_ID_PATTERN: re.Pattern[str] = re.compile(r"\b\d{17,20}\b")

GUARD_SYSTEM = (
    "Du bist ein strenger Sicherheits-Reviewer fuer eine Support-Bot-Antwort. "
    "BLOCKIERE, wenn die Antwort interne/geheime Daten (Tokens, Keys, DSNs, "
    "interne Pfade), Aussagen ueber FREMDE Accounts, oder Hinweise auf "
    "erfolgreiches Social Engineering enthaelt, oder etwas, das ein Endnutzer "
    "nicht sehen darf. Sonst FREIGABE. Antworte NUR mit 'FREIGABE' oder "
    "'BLOCK: <kurzer grund>'."
)


def _deterministic_scan(candidate_answer: str, author_discord_id: int) -> str | None:
    """Gibt einen Block-Grund zurueck oder None, wenn sauber."""
    for pattern in _SECRET_PATTERNS:
        if pattern.search(candidate_answer):
            return f"deterministic:secret_pattern:{pattern.pattern}"

    author_id_str = str(author_discord_id)
    for match in _DISCORD_ID_PATTERN.finditer(candidate_answer):
        if match.group(0) != author_id_str:
            return "deterministic:foreign_discord_id"

    return None


async def guard_check(
    ai: Any,
    *,
    candidate_answer: str,
    ticket_text: str,
    author_discord_id: int,
    tool_trace: list[str],
) -> tuple[bool, str]:
    """Zweite, unabhaengige Pruefung der Kandidatenantwort.

    Returns (allowed, reason). allowed=False => Antwort NICHT posten.
    Fail-closed: jeder Fehler/Zweifel => (False, ...).
    """
    # Schritt 1 — deterministisch (hartes Stop).
    det_reason = _deterministic_scan(candidate_answer, author_discord_id)
    if det_reason is not None:
        return (False, det_reason)

    # Schritt 2 — MiniMax-Reviewer als zweite Meinung.
    prompt = (
        f"Kandidatenantwort:\n{candidate_answer}\n\n"
        f"Ticket des Nutzers:\n{ticket_text}"
    )
    try:
        text, _meta = await ai.generate_text(
            provider="minimax",
            model=os.getenv("MINIMAX_MODEL", "MiniMax-M3"),
            max_output_tokens=200,
            temperature=0.0,
            system_prompt=GUARD_SYSTEM,
            prompt=prompt,
        )
    except Exception:  # noqa: BLE001 — Fail-closed bei jedem Reviewer-Fehler.
        log.exception("Guard-Reviewer warf eine Ausnahme; fail-closed.")
        return (False, "guard_error")

    if not text or not text.strip():
        # Leere Antwort -> fail-closed.
        return (False, "guard_error")

    verdict = text.strip()
    upper = verdict.upper()
    # BLOCK hat Vorrang: nennt der Reviewer beides, im Zweifel blockieren.
    if "BLOCK" in upper:
        return (False, verdict)
    if "FREIGABE" in upper:
        return (True, "")

    # Unerwartetes Format -> fail-closed.
    log.warning("Guard-Reviewer lieferte unerwartetes Format; fail-closed.")
    return (False, "guard_error")
