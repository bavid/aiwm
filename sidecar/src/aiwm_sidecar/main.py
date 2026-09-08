"""Sidecar entry point — Phase 1 stub.

Speaks JSON-RPC 2.0 over stdio, one JSON object per line. Fully implemented in
WP-8; for now it answers only ``handshake`` and ``ping`` so the Rust
``SidecarClient`` can be wired up and tested. ``inspect_model_file`` is declared
in the contract but not served until Phase 2.
"""

from __future__ import annotations

import json
import sys
from typing import Any

from aiwm_sidecar import PROTOCOL_VERSION, __version__

CAPABILITIES: list[str] = []  # e.g. "inspect_model_file" from Phase 2 onward

_METHOD_NOT_FOUND = -32601
_NOT_IMPLEMENTED = -32001

# Methods declared in the contract but not served yet.
_PLANNED = {"inspect_model_file"}


def _error(req_id: Any, code: int, message: str) -> dict[str, Any]:
    return {"jsonrpc": "2.0", "id": req_id, "error": {"code": code, "message": message}}


def handle(req: dict[str, Any]) -> dict[str, Any] | None:
    """Turn one JSON-RPC request into one response (or None for notifications)."""
    method = req.get("method")
    req_id = req.get("id")

    if method == "handshake":
        result: Any = {
            "sidecar_version": __version__,
            "protocol_version": PROTOCOL_VERSION,
            "capabilities": CAPABILITIES,
        }
    elif method == "ping":
        result = "pong"
    elif method in _PLANNED:
        return _error(req_id, _NOT_IMPLEMENTED, f"{method} is not implemented yet")
    else:
        return _error(req_id, _METHOD_NOT_FOUND, f"method not found: {method}")

    if req_id is None:
        return None
    return {"jsonrpc": "2.0", "id": req_id, "result": result}


def main() -> None:
    for raw in sys.stdin:
        raw = raw.strip()
        if not raw:
            continue
        try:
            req = json.loads(raw)
        except json.JSONDecodeError:
            continue
        resp = handle(req)
        if resp is not None:
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
