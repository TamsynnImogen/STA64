#!/usr/bin/env python3
"""Small stdio MCP bridge to a running STA64 agent-control endpoint.

The endpoint file is created by the game. This process only connects to
127.0.0.1, and keeps the game's control lease private to this MCP session.
"""

import argparse
import json
import pathlib
import socket
import sys
import threading


VERSIONS = ("2025-11-25", "2025-06-18", "2024-11-05")
HEARTBEAT_SECONDS = 20
MAX_REPLY_BYTES = 16 * 1024 * 1024


def tool(name, description, properties=None, required=None):
    return {
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object", "properties": properties or {},
            "required": required or [], "additionalProperties": False,
        },
    }


TOOLS = [
    tool("status", "Read the live game and control endpoint status."),
    tool("claim", "Claim the authorized player seat in the local match. The lease remains private to this MCP session."),
    tool("observe", "Read live entities. 'after' is an optional entity ID cursor; 'limit' bounds the reply.",
         {"after": {"type": "integer", "minimum": 0, "maximum": 4294967295},
          "limit": {"type": "integer", "minimum": 1, "maximum": 100}}),
    tool("order", "Issue an order after claiming control. The game validates required fields and legality. Move/move_warp require units and position; attack/mine require units and target; stop/scout/search_and_destroy require units; build requires builder and odf (position optional); rally requires units and optionally target or position; special requires units and slot (target or position optional); repair requires units (target optional).",
         {"action": {"type": "string", "enum": ["move", "move_warp", "attack", "mine", "stop", "build", "scout", "search_and_destroy", "rally", "special", "repair"]},
          "units": {"type": "array", "items": {"type": "integer", "minimum": 1}, "minItems": 1},
          "target": {"type": "integer"},
          "position": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
          "builder": {"type": "integer"}, "odf": {"type": "string"},
          "queue": {"type": "boolean"}, "slot": {"type": "integer"}},
         ["action"]),
    tool("release", "Release this MCP session's game-control lease."),
]
TOOL_NAMES = {entry["name"] for entry in TOOLS}

class LeaseExpired(RuntimeError):
    pass


class GameBridge:
    def __init__(self, endpoint):
        self.endpoint = pathlib.Path(endpoint)
        self.lease = None
        self.lock = threading.Lock()
        self.done = threading.Event()
        self.heartbeat = None

    def _exchange(self, op, arguments=None, lease=None):
        endpoint = json.loads(self.endpoint.read_text(encoding="utf-8"))
        port, token = endpoint["port"], endpoint["token"]
        if type(port) is not int or not 1 <= port <= 65535 or not isinstance(token, str) or not token:
            raise ValueError("invalid game endpoint file")
        request = {"token": token, "op": op}
        if lease is not None:
            request["lease"] = lease
        if arguments is not None:
            request["arguments"] = arguments
        with socket.create_connection(("127.0.0.1", port), timeout=5) as connection:
            connection.settimeout(5)
            connection.sendall((json.dumps(request, separators=(",", ":")) + "\n").encode("utf-8"))
            with connection.makefile("rb") as stream:
                reply = stream.readline(MAX_REPLY_BYTES + 1)
        if not reply or len(reply) > MAX_REPLY_BYTES or not reply.endswith(b"\n"):
            raise ValueError("missing or oversized game reply")
        response = json.loads(reply)
        if not isinstance(response, dict) or type(response.get("ok")) is not bool:
            raise ValueError("invalid game reply")
        if not response["ok"]:
            error = str(response.get("error", "game operation failed"))
            if error in ("No active seat lease. Claim the seat first.",
                         "This seat lease is no longer valid. Claim the seat again.",
                         "Start an eligible local match with the authorized seat present."):
                raise LeaseExpired(error)
            raise RuntimeError(error)
        return response.get("data")

    def call(self, op, arguments):
        with self.lock:
            if op == "claim" and self.lease is not None:
                try:
                    self._exchange("keepalive", {}, self.lease)
                except LeaseExpired:
                    self.lease = None
                else:
                    raise RuntimeError("this MCP session already holds a control lease")
            if op == "release" and self.lease is None:
                raise RuntimeError("this MCP session has no control lease")
            try:
                data = self._exchange(op, arguments, self.lease)
            except LeaseExpired:
                self.lease = None
                raise
            if op == "claim":
                if not isinstance(data, dict) or not isinstance(data.get("lease"), str) or not data["lease"]:
                    raise ValueError("claim reply has no lease")
                self.lease = data["lease"]
                data = {key: value for key, value in data.items() if key != "lease"}
            elif op == "release":
                self.lease = None
            return data

    def start_heartbeat(self):
        def run():
            while not self.done.wait(HEARTBEAT_SECONDS):
                with self.lock:
                    if self.lease is not None:
                        try:
                            self._exchange("keepalive", {}, self.lease)
                        except LeaseExpired:
                            self.lease = None
                        except (OSError, ValueError, RuntimeError, KeyError, json.JSONDecodeError):
                            # A subsequent tool call reports endpoint errors to the client.
                            pass

        self.heartbeat = threading.Thread(target=run, name="sta64-lease-heartbeat", daemon=True)
        self.heartbeat.start()

    def close(self):
        self.done.set()
        with self.lock:
            if self.lease is not None:
                try:
                    self._exchange("release", {}, self.lease)
                except (OSError, ValueError, RuntimeError, KeyError, json.JSONDecodeError):
                    pass
                self.lease = None
        if self.heartbeat is not None:
            self.heartbeat.join(timeout=6)


class McpServer:
    def __init__(self, bridge):
        self.bridge = bridge
        self.initialized = False

    def handle(self, message):
        if not isinstance(message, dict) or message.get("jsonrpc") != "2.0":
            return self.error(None, -32600, "Invalid Request")
        method = message.get("method")
        if not isinstance(method, str):
            return None if "id" not in message else self.error(message["id"], -32600, "Invalid Request")
        if "id" not in message:
            if method == "notifications/initialized" and self.initialized:
                return None
            return None
        request_id = message["id"]
        params = message.get("params", {})
        if not isinstance(params, dict):
            return self.error(request_id, -32602, "Invalid params")
        if method == "initialize":
            requested = params.get("protocolVersion")
            if not isinstance(requested, str):
                return self.error(request_id, -32602, "Missing protocolVersion")
            self.initialized = True
            version = requested if requested in VERSIONS else VERSIONS[0]
            return self.result(request_id, {"protocolVersion": version,
                                            "capabilities": {"tools": {}},
                                            "serverInfo": {"name": "sta64-mcp", "version": "0.1.0"}})
        if method == "ping":
            return self.result(request_id, {})
        if not self.initialized:
            return self.error(request_id, -32000, "Initialize first")
        if method == "tools/list":
            return self.result(request_id, {"tools": TOOLS})
        if method == "tools/call":
            name, arguments = params.get("name"), params.get("arguments", {})
            if not isinstance(name, str) or name not in TOOL_NAMES or not isinstance(arguments, dict):
                return self.error(request_id, -32602, "Unknown tool or invalid arguments")
            try:
                data = self.bridge.call(name, arguments)
                return self.result(request_id, {"content": [{"type": "text", "text": json.dumps(data)}],
                                                "structuredContent": {"data": data}})
            except (OSError, ValueError, RuntimeError, KeyError, json.JSONDecodeError) as exc:
                return self.result(request_id, {"content": [{"type": "text", "text": str(exc)}],
                                                "isError": True})
        return self.error(request_id, -32601, "Method not found")

    @staticmethod
    def result(request_id, value):
        return {"jsonrpc": "2.0", "id": request_id, "result": value}

    @staticmethod
    def error(request_id, code, message):
        return {"jsonrpc": "2.0", "id": request_id,
                "error": {"code": code, "message": message}}


def main(argv=None, stdin=None, stdout=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", required=True, type=pathlib.Path,
                        help="Path to the game-created JSON endpoint file")
    args = parser.parse_args(argv)
    bridge = GameBridge(args.endpoint)
    bridge.start_heartbeat()
    server = McpServer(bridge)
    stdin = stdin or sys.stdin
    stdout = stdout or sys.stdout
    try:
        for line in stdin:
            try:
                message = json.loads(line)
                reply = server.handle(message)
            except (ValueError, json.JSONDecodeError):
                reply = server.error(None, -32700, "Parse error")
            if reply is not None:
                stdout.write(json.dumps(reply, separators=(",", ":")) + "\n")
                stdout.flush()
    finally:
        bridge.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
