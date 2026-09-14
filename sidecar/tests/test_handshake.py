from aiwm_sidecar.main import handle


def test_handshake_returns_versions_and_capabilities():
    resp = handle({"jsonrpc": "2.0", "id": 1, "method": "handshake"})
    assert resp is not None
    assert resp["result"]["protocol_version"] == 1
    assert isinstance(resp["result"]["capabilities"], list)
    assert "synthesize_speech" in resp["result"]["capabilities"]


def test_ping_returns_pong():
    resp = handle({"jsonrpc": "2.0", "id": 2, "method": "ping"})
    assert resp is not None
    assert resp["result"] == "pong"


def test_unknown_method_is_json_rpc_error():
    resp = handle({"jsonrpc": "2.0", "id": 3, "method": "nope"})
    assert resp is not None
    assert resp["error"]["code"] == -32601


def test_planned_method_reports_not_implemented():
    resp = handle({"jsonrpc": "2.0", "id": 4, "method": "inspect_model_file"})
    assert resp is not None
    assert resp["error"]["code"] == -32001


def test_notification_without_id_yields_no_response():
    assert handle({"jsonrpc": "2.0", "method": "ping"}) is None
