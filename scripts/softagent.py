"""A small client for soft-agent's MCP tools, for the scripts here.

Starts soft-agent as a child process and calls its tools over MCP (JSON-RPC
over stdin/stdout), as any MCP client would, but with no AI agent: each
script decides what to call.

    with SoftAgent(server, engine_folder) as sa:
        out = sa.tool("get_soft_agent_info", {})
"""

import json
import subprocess
import sys


class ToolFailed(Exception):
    """A tool's error, as the engine returns it: category, message and
    details."""

    def __init__(self, tool, error):
        self.tool = tool
        self.error = error if isinstance(error, dict) else {"message": str(error)}
        super().__init__(f"{tool}: {self.error.get('message', error)}")


class SoftAgent:
    def __init__(self, server, engine_folder, client_name="soft-agent-script"):
        self.proc = subprocess.Popen(
            [server, "--engine-folder", engine_folder],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
        )
        self.last_id = 0
        self._request("initialize", {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": client_name, "version": "1"},
        })
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    def _send(self, msg):
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()

    def _request(self, method, params):
        self.last_id += 1
        self._send({"jsonrpc": "2.0", "id": self.last_id, "method": method, "params": params})
        while True:
            line = self.proc.stdout.readline()
            if not line:
                sys.exit("soft-agent stopped unexpectedly.")
            msg = json.loads(line)
            if msg.get("id") == self.last_id:
                if "error" in msg:
                    raise ToolFailed(method, msg["error"])
                return msg.get("result", {})

    def tool(self, name, arguments):
        """Calls a tool; returns its result, or raises ToolFailed."""
        result = self._request("tools/call", {"name": name, "arguments": arguments})
        out = result.get("structuredContent", result)
        if result.get("isError"):
            raise ToolFailed(name, out)
        return out

    def close(self):
        if self.proc.poll() is None:
            self.proc.stdin.close()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()


def board_port(sa, build_folder=None):
    """The serial port of the one connected board (that fits the build, when
    given), and its device id; exits with a message when there isn't
    exactly one."""
    args = {"build_folder": build_folder} if build_folder else {}
    devices = [d for d in sa.tool("list_devices", args).get("devices", [])
               if d.get("serial_port") and (build_folder or d.get("likely_boards"))]
    if not devices:
        sys.exit("No board found: connect the board with its USB cable and try again.")
    if len(devices) > 1:
        names = ", ".join(f"{d['serial_port']} ({d.get('who', d['id'])})" for d in devices)
        sys.exit(f"Several boards are connected: {names}. Give one with --port.")
    return devices[0]["serial_port"], devices[0]["id"]
