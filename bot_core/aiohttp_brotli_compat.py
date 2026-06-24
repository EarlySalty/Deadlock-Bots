"""aiohttp-Antworten auf gzip/deflate beschraenken (kein Brotli).

aiohttp 3.13/3.14 reicht beim Streaming-Decompress ein ``max_length`` an den
Brotli-Decoder durch (DoS-Schutz). Weder das System-``Brotli``- noch das
``brotlicffi``-Backend akzeptieren dieses Argument positional ->
``ContentEncodingError: Can not decode content-encoding: br`` auf den
sporadisch Brotli-komprimierten Discord-Antworten. Das schlaegt u. a. beim
Senden der Leave-Survey-DM mit einem 400-Traceback durch.

Da gzip/deflate von aiohttp korrekt verarbeitet werden und ``br`` nur
optionale Bandbreitenersparnis bringt, nehmen wir ``br`` aus dem
Accept-Encoding-Default. Kein aiohttp-Downgrade (3.14.0 schliesst 4 CVEs),
keine neue Abhaengigkeit. Idempotent.
"""

from __future__ import annotations

import logging

logger = logging.getLogger(__name__)


def apply() -> None:
    """Entfernt ``br`` aus aiohttps Default-Accept-Encoding (idempotent)."""
    try:
        from aiohttp import hdrs
        from aiohttp.client_reqrep import ClientRequest
    except Exception:  # aiohttp nicht verfuegbar -> nichts zu tun
        return
    headers = getattr(ClientRequest, "DEFAULT_HEADERS", None)
    if headers is None:
        return
    current = headers.get(hdrs.ACCEPT_ENCODING, "")
    encodings = [
        part.strip()
        for part in current.split(",")
        if part.strip() and part.strip().lower() != "br"
    ]
    if not encodings:
        encodings = ["gzip", "deflate"]
    new_value = ", ".join(encodings)
    if new_value != current:
        headers[hdrs.ACCEPT_ENCODING] = new_value
        logger.info(
            "aiohttp Accept-Encoding auf '%s' gesetzt (Brotli deaktiviert)",
            new_value,
        )


apply()
