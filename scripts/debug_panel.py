"""Opens the debug panel for a debug build on the board, from the shell.

Run it through scripts/debug-panel.sh, which builds soft-agent first:

    scripts/debug-panel.sh BUILD_FOLDER [--port PORT] [--no-open]

It starts soft-agent, opens a debug session (debug_open) and the panel
(debug_panel), and waits; Enter or Ctrl-C closes the session, which releases
every override and frees the port. Without --port it uses the serial port
of the one connected board list_devices finds.
"""

import argparse
import json
import os
import subprocess
import sys


def main():
    ap = argparse.ArgumentParser(description="Open the soft-agent debug panel.")
    ap.add_argument("build_folder", help="the debug build's folder, such as ~/my-design/_build_debug")
    ap.add_argument("--port", help="the board's serial port; found by list_devices when left out")
    ap.add_argument("--no-open", action="store_true", help="print the panel's address without opening it")
    ap.add_argument("--server", required=True, help="the soft-agent binary (debug-panel.sh passes it)")
    ap.add_argument("--engine-folder", required=True, help="the engine folder (debug-panel.sh passes it)")
    args = ap.parse_args()

    server = subprocess.Popen(
        [args.server, "--engine-folder", args.engine_folder],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
    )
    last_id = 0

    def request(method, params):
        nonlocal last_id
        last_id += 1
        send({"jsonrpc": "2.0", "id": last_id, "method": method, "params": params})
        while True:
            line = server.stdout.readline()
            if not line:
                sys.exit("soft-agent stopped.")
            msg = json.loads(line)
            if msg.get("id") == last_id:
                return msg.get("result", {})

    def send(msg):
        server.stdin.write(json.dumps(msg) + "\n")
        server.stdin.flush()

    def tool(name, arguments):
        result = request("tools/call", {"name": name, "arguments": arguments})
        out = result.get("structuredContent", result)
        if result.get("isError"):
            sys.exit(f"{name}: {out.get('message', out)}")
        return out

    request("initialize", {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "debug-panel", "version": "1"},
    })
    send({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})

    port = args.port
    if not port:
        ports = [d["serial_port"] for d in tool("list_devices", {})["devices"]
                 if d.get("serial_port") and d.get("likely_boards")]
        if len(ports) != 1:
            sys.exit(f"Found {len(ports)} board serial ports {ports}: give one with --port.")
        port = ports[0]

    folder = os.path.abspath(os.path.expanduser(args.build_folder))
    s = tool("debug_open", {"build_folder": folder, "port": port})
    print(f"Session {s['session']} on {port}, build {s['build_id']}")
    panel = tool("debug_panel", {"session": s["session"], "open": not args.no_open})
    print(f"Panel: {panel['url']}")
    try:
        input("Press Enter to close the session (or Ctrl-C)... ")
    except (KeyboardInterrupt, EOFError):
        print()
    finally:
        released = tool("debug_close", {"session": s["session"]})["released"]
        print("Closed; overrides released." if released else "Closed; couldn't release the overrides.")
        server.stdin.close()
        server.wait(timeout=10)


if __name__ == "__main__":
    main()
