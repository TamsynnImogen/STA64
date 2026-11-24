import importlib.util
import io
import json
import pathlib
import socketserver
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock


SCRIPT = pathlib.Path(__file__).with_name("sta64_mcp.py")
SPEC = importlib.util.spec_from_file_location("sta64_mcp", SCRIPT)
mcp = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mcp)


class GameHandler(socketserver.StreamRequestHandler):
    def handle(self):
        request = json.loads(self.rfile.readline())
        with self.server.requests_lock:
            self.server.requests.append(request)
        op = request["op"]
        if op == "claim":
            response = {"ok": True, "data": {"lease": "private-lease", "claimed": True}}
        elif op in ("order", "observe", "keepalive", "release") and request.get("lease") != "private-lease":
            response = {"ok": False, "error": "missing lease"}
        else:
            response = {"ok": True, "data": {"operation": op}}
        self.wfile.write((json.dumps(response) + "\n").encode())


class GameServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


class ProxyTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.server = GameServer(("127.0.0.1", 0), GameHandler)
        self.server.requests = []
        self.server.requests_lock = threading.Lock()
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.endpoint = pathlib.Path(self.tmp.name, "endpoint.json")
        self.endpoint.write_text(json.dumps({"port": self.server.server_address[1], "token": "private-token"}))

    def test_mcp_handshake_tools_and_eof_release(self):
        requests = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "ping"},
            {"jsonrpc": "2.0", "id": 3, "method": "tools/list"},
            {"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "claim"}},
            {"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "observe", "arguments": {"after": 7, "limit": 2}}},
            {"jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": {"name": "order", "arguments": {"action": "move", "units": [1], "position": [1, 2, 3]}}},
        ]
        output = io.StringIO()
        self.assertEqual(mcp.main(["--endpoint", str(self.endpoint)], io.StringIO("".join(json.dumps(r) + "\n" for r in requests)), output), 0)
        replies = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual([reply["id"] for reply in replies], [1, 2, 3, 4, 5, 6])
        self.assertEqual(replies[0]["result"]["protocolVersion"], "2025-11-25")
        self.assertEqual({tool["name"] for tool in replies[2]["result"]["tools"]}, {"status", "claim", "observe", "order", "release"})
        self.assertNotIn("private-lease", output.getvalue())
        self.assertNotIn("private-token", output.getvalue())
        self.assertEqual([r["op"] for r in self.server.requests], ["claim", "observe", "order", "release"])
        self.assertEqual(self.server.requests[1]["arguments"], {"after": 7, "limit": 2})
        self.assertEqual(self.server.requests[2]["lease"], "private-lease")
        self.assertTrue(all(r["token"] == "private-token" for r in self.server.requests))

    def test_heartbeat_and_explicit_release(self):
        bridge = mcp.GameBridge(self.endpoint)
        with mock.patch.object(mcp, "HEARTBEAT_SECONDS", 0.03):
            bridge.start_heartbeat()
            try:
                self.assertEqual(bridge.call("claim", {}), {"claimed": True})
                deadline = time.monotonic() + 2
                while not any(r["op"] == "keepalive" for r in self.server.requests) and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue(any(r["op"] == "keepalive" and r["arguments"] == {} for r in self.server.requests))
                bridge.call("release", {})
            finally:
                bridge.close()
        self.assertEqual([r["op"] for r in self.server.requests].count("release"), 1)
        self.assertIsNone(bridge.lease)

    def test_backward_version_and_game_error(self):
        server = mcp.McpServer(mcp.GameBridge(self.endpoint))
        for version in ("2025-06-18", "2024-11-05"):
            result = server.handle({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": version}})
            self.assertEqual(result["result"]["protocolVersion"], version)
        result = server.handle({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "order", "arguments": {"action": "stop"}}})
        self.assertTrue(result["result"]["isError"])
        self.assertIn("missing lease", result["result"]["content"][0]["text"])


def exercise_live_game(endpoint, other_id, own_id):
    """Drive a real game through the stdio proxy from the Rust integration test."""
    process = subprocess.Popen(
        [sys.executable, str(SCRIPT), "--endpoint", endpoint],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
    )
    request_id = 0

    def rpc(method, params):
        nonlocal request_id
        request_id += 1
        print(json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}),
              file=process.stdin, flush=True)
        reply = json.loads(process.stdout.readline())
        assert reply["id"] == request_id, reply
        return reply["result"]

    def call(name, arguments=None, *, expect_error=False):
        result = rpc("tools/call", {"name": name, "arguments": arguments or {}})
        if expect_error:
            assert result.get("isError"), result
            return
        assert not result.get("isError"), result
        return result["structuredContent"]["data"]

    try:
        rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                           "clientInfo": {"name": "STA64 integration test", "version": "1"}})
        print(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}),
              file=process.stdin, flush=True)
        assert call("status")["available"]
        call("observe", expect_error=True)
        assert call("claim")["seat"] == 1
        units = call("observe")["units"]
        before = next(ship["position"] for ship in units if ship["id"] == own_id)
        assert all(row["id"] != other_id for row in units)
        assert not call("observe", {"after": own_id, "limit": 1})["units"]
        call("order", {"action": "stop", "units": [other_id]}, expect_error=True)
        destination = [before[0], before[1], before[2] + 500]
        queued = call("order", {"action": "move", "units": [own_id], "position": destination})
        deadline = time.monotonic() + 5
        while True:
            observation = call("observe")
            results = observation["recent_orders"]
            moved = next(row for row in observation["units"] if row["id"] == own_id)
            if results and moved["position"] != before:
                assert results[-1]["sequence"] == queued["sequence"] and results[-1]["accepted"]
                break
            assert time.monotonic() < deadline, "ship did not move"
            time.sleep(0.02)
        assert call("release")["released"]
        assert not call("status")["controlled"]
    finally:
        process.stdin.close()
        try:
            assert process.wait(timeout=8) == 0
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            raise
        process.stdout.close()


if __name__ == "__main__":
    if sys.argv[1:2] == ["--live-game"]:
        endpoint, other, own = sys.argv[2:]
        exercise_live_game(endpoint, int(other), int(own))
    else:
        unittest.main()
