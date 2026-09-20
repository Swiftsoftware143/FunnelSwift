#!/usr/bin/env python3
"""Host-side Tesseract OCR service for FunnelSwift (docker-bridge only).

Mirrors the `rust-compiler-guard.service` pattern on this box (172.17.0.1:8092):
a tiny stdlib-only HTTP service bound to the docker bridge that containers call
for work the app image cannot do itself. Tesseract 5.x is installed on the HOST
but not inside the `funnelswift` image, and the image is a live container.

Endpoint:
    POST /ocr        (header: X-OCR-Token)
    {"image_base64": "<base64 | data:image/png;base64,...>"}  ->  {"text": "..."}

Design notes:
  * stdlib only (PEP 668 blocks pip installs on this box).
  * binds ONLY the docker bridge address 172.17.0.1 — never 0.0.0.0, so the
    service is not publicly reachable.
  * image is decoded to a unique 0600 temp file, OCR'd via subprocess argv
    (no shell=True) with a hard timeout, and removed in a finally block.
  * logs to a file, never stdout.
  * refuses to start when the shared-secret file is missing.
"""
from __future__ import annotations

import base64
import binascii
import hmac
import json
import logging
import os
import subprocess
import tempfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

BIND_HOST = "172.17.0.1"
BIND_PORT = 8093
MAX_B64_CHARS = 8_000_000
TESSERACT = "/usr/bin/tesseract"
OCR_TIMEOUT_S = 20
MAX_BODY_BYTES = MAX_B64_CHARS + 4096
TOKEN_FILE = "/root/.ocr_service_token"
LOG_FILE = "/var/log/funnelswift-ocr.log"

log = logging.getLogger("funnelswift-ocr")


def _load_token() -> str:
    """Read the shared secret. Missing/empty -> refuse to start."""
    if not os.path.exists(TOKEN_FILE):
        raise SystemExit(
            f"FATAL: token file {TOKEN_FILE} missing — refusing to start "
            "(create it with: openssl rand -hex 32 > %s; chmod 600 %s)"
            % (TOKEN_FILE, TOKEN_FILE)
        )
    with open(TOKEN_FILE, "r", encoding="utf-8") as fh:
        token = fh.read().strip()
    if not token:
        raise SystemExit(f"FATAL: token file {TOKEN_FILE} is empty — refusing to start")
    return token


def _strip_data_uri(value: str) -> str:
    """Drop an optional `data:image/png;base64,` prefix and whitespace."""
    if value.startswith("data:"):
        comma = value.find(",")
        if comma != -1:
            value = value[comma + 1 :]
    return "".join(value.split())


def run_tesseract(image_bytes: bytes) -> str:
    """Decode to a unique 0600 temp file, OCR it, always delete the file."""
    fd, path = tempfile.mkstemp(prefix="fs_ocr_", suffix=".img")
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "wb") as fh:
            fh.write(image_bytes)
        proc = subprocess.run(  # noqa: S603 - argv list, no shell
            [TESSERACT, path, "-", "--psm", "6"],
            capture_output=True,
            timeout=OCR_TIMEOUT_S,
            check=False,
        )
        if proc.returncode != 0:
            err = proc.stderr.decode("utf-8", "replace").strip()[:300]
            raise RuntimeError(f"tesseract exit {proc.returncode}: {err}")
        return proc.stdout.decode("utf-8", "replace")
    finally:
        try:
            os.unlink(path)
        except OSError:
            pass


class Handler(BaseHTTPRequestHandler):
    server_version = "FunnelSwiftOCR/1.0"
    token = ""

    def log_message(self, fmt, *args):  # route HTTP noise to the log file
        log.info("%s - %s", self.address_string(), fmt % args)

    def _reply(self, code: int, payload: dict) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):  # health probe, no secrets involved
        if self.path == "/health":
            self._reply(200, {"status": "ok"})
        else:
            self._reply(404, {"error": "not found"})

    def do_POST(self):
        if self.path != "/ocr":
            self._reply(404, {"error": "not found"})
            return

        supplied = self.headers.get("X-OCR-Token", "")
        if not supplied or not hmac.compare_digest(supplied, self.token):
            log.warning("rejected request from %s: bad/missing token", self.address_string())
            self._reply(401, {"error": "unauthorized"})
            return

        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            self._reply(400, {"error": "invalid Content-Length"})
            return
        if length <= 0:
            self._reply(400, {"error": "empty request body"})
            return
        if length > MAX_BODY_BYTES:
            self._reply(413, {"error": "request body too large"})
            return

        raw = self.rfile.read(length)
        try:
            payload = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            self._reply(400, {"error": "invalid JSON"})
            return

        image_b64 = payload.get("image_base64") if isinstance(payload, dict) else None
        if not isinstance(image_b64, str) or not image_b64.strip():
            self._reply(400, {"error": "image_base64 is required"})
            return

        cleaned = _strip_data_uri(image_b64)
        if not cleaned:
            self._reply(400, {"error": "image_base64 decodes to nothing"})
            return
        if len(cleaned) > MAX_B64_CHARS:
            self._reply(413, {"error": "image_base64 too large"})
            return

        try:
            image_bytes = base64.b64decode(cleaned, validate=True)
        except (binascii.Error, ValueError):
            self._reply(400, {"error": "image_base64 is not valid base64"})
            return
        if not image_bytes:
            self._reply(400, {"error": "image_base64 is empty"})
            return

        try:
            text = run_tesseract(image_bytes)
        except subprocess.TimeoutExpired:
            log.error("tesseract timed out after %ss", OCR_TIMEOUT_S)
            self._reply(504, {"error": "ocr timeout"})
            return
        except Exception as exc:  # noqa: BLE001 - report and keep serving
            log.error("ocr failed: %s", exc)
            self._reply(500, {"error": "ocr failed", "detail": str(exc)[:200]})
            return

        log.info("ocr ok: %d bytes in, %d chars out", len(image_bytes), len(text))
        self._reply(200, {"text": text})


def main() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s",
        filename=LOG_FILE,
    )
    Handler.token = _load_token()
    server = ThreadingHTTPServer((BIND_HOST, BIND_PORT), Handler)
    log.info("funnelswift-ocr listening on %s:%d", BIND_HOST, BIND_PORT)
    server.serve_forever()


if __name__ == "__main__":
    main()
