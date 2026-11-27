"""A small client for soft-agent's MCP tools, for the scripts here.

Starts soft-agent as a child process and calls its tools over MCP (JSON-RPC
over stdin/stdout), as any MCP client would, but with no AI agent: each
script decides what to call.

    with SoftAgent(server, engine_folder) as sa:
        out = sa.tool("get_soft_agent_info", {})
"""

import json
import os
import subprocess
import sys


def default_server():
    """The repo's release build: target/release/soft-agent (.exe on
    Windows), as `cargo build --release` makes it."""
    exe = "soft-agent.exe" if os.name == "nt" else "soft-agent"
    repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    return os.path.join(repo, "target", "release", exe)


def default_engine_folder():
    """The scripts' engine folder, as scripts/_rust-env.sh has it:
    $SOFT_AGENT_DATA_FOLDER, else $SOFT_AGENT_HOME/engine-data, else
    ~/.logix/soft-agent/engine-data."""
    if os.environ.get("SOFT_AGENT_DATA_FOLDER"):
        return os.environ["SOFT_AGENT_DATA_FOLDER"]
    home = os.environ.get("SOFT_AGENT_HOME") or os.path.join(os.path.expanduser("~"), ".logix", "soft-agent")
    return os.path.join(home, "engine-data")


class ToolFailed(Exception):
    """A tool's error, as the engine returns it: category, message and
    details."""

    def __init__(self, tool, error):
        self.tool = tool
        self.error = error if isinstance(error, dict) else {"message": str(error)}
        super().__init__(f"{tool}: {self.error.get('message', error)}")


class SoftAgent:
    def __init__(self, server, engine_folder, client_name="soft-agent-script"):
        if not os.path.isfile(server):
            sys.exit(f"No soft-agent at {server}: build it first, with `cargo build --release` in the repo.")
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


def board_port(sa, build_folder=None, port=None):
    """The serial port of the one connected board (that fits the build, when
    given), and its device id; exits with a message when there isn't
    exactly one. With `port` (the user's --port), the board needn't have a
    serial port in list_devices: that port is used, and the board found is
    the one to program."""
    args = {"build_folder": build_folder} if build_folder else {}
    found = sa.tool("list_devices", args)

    def fits(d):
        # With a build folder, list_devices lists only the devices that fit it.
        return build_folder or d.get("likely_boards")

    if port:
        devices = [d for d in found.get("devices", []) if fits(d)]
        if len(devices) > 1:
            # Several boards: the one with this port, when the engine knows it.
            mine = [d for d in devices
                    if any(p.get("port") == port for p in d.get("serial_ports", []))]
            devices = mine or devices
    else:
        devices = [d for d in found.get("devices", []) if d.get("serial_port") and fits(d)]
    if not devices:
        # Seen but not openable, such as on Linux before its udev rule.
        for u in found.get("unreachable", []):
            print(u.get("note", f"{u['id']} can't be opened."), file=sys.stderr)
            print(f"  To fix it: python scripts/tool.py install_driver device={u['id']}", file=sys.stderr)
        if found.get("unreachable"):
            sys.exit("The board can't be opened yet: see above.")
        if not port and any(fits(d) for d in found.get("devices", [])):
            sys.exit("The board is connected, but list_devices gives it no serial port: "
                     "give its port with --port (on Windows, Device Manager lists it under Ports).")
        sys.exit("No board found: connect the board with its USB cable and try again.")
    if len(devices) > 1:
        if port:
            names = ", ".join(d.get("who", d["id"]) for d in devices)
            sys.exit(f"Several boards are connected ({names}), and none is known to have {port}: "
                     "connect only the one to check.")
        names = ", ".join(f"{d['serial_port']} ({d.get('who', d['id'])})" for d in devices)
        sys.exit(f"Several boards are connected: {names}. Give one with --port.")
    return port or devices[0]["serial_port"], devices[0]["id"]
