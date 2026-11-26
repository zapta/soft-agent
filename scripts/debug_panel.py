"""Opens the debug panel for a debug build on the board, from the shell.

Run it through scripts/debug-panel.sh, which builds soft-agent first:

    scripts/debug-panel.sh BUILD_FOLDER [--port PORT] [--no-open]

It starts soft-agent, opens a debug session (debug_open) and the panel
(debug_panel), and waits; Enter or Ctrl-C closes the session, which releases
every override and frees the port. Without --port it uses the serial port
of the one connected board that fits the build.
"""

import argparse
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from softagent import SoftAgent, ToolFailed, board_port  # noqa: E402


def open_session(sa, build_folder, port, wait=0):
    """debug_open, trying again for up to `wait` seconds while the debugger
    doesn't answer, as while an FPGA loads its design from flash."""
    deadline = time.monotonic() + wait
    while True:
        try:
            return sa.tool("debug_open", {"build_folder": build_folder, "port": port})
        except ToolFailed as e:
            if "didn't answer" not in e.error.get("message", "") or time.monotonic() > deadline:
                raise
            time.sleep(1)


def open_panel(sa, build_folder, port, open_browser=True, wait=0):
    """Opens a session and the panel, waits for Enter or Ctrl-C, then
    closes the session. `wait` is as for open_session."""
    s = open_session(sa, build_folder, port, wait)
    print(f"Session {s['session']} on {port}, build {s['build_id']}")
    panel = sa.tool("debug_panel", {"session": s["session"], "open": open_browser})
    print(f"Panel: {panel['url']}")
    try:
        input("Press Enter to close the session (or Ctrl-C)... ")
    except (KeyboardInterrupt, EOFError):
        print()
    finally:
        released = sa.tool("debug_close", {"session": s["session"]})["released"]
        print("Closed; overrides released." if released else "Closed; couldn't release the overrides.")


def main():
    ap = argparse.ArgumentParser(description="Open the soft-agent debug panel.")
    ap.add_argument("build_folder", help="the debug build's folder, such as ~/my-design/_build_debug")
    ap.add_argument("--port", help="the board's serial port; found by list_devices when left out")
    ap.add_argument("--no-open", action="store_true", help="print the panel's address without opening it")
    ap.add_argument("--server", required=True, help="the soft-agent binary (debug-panel.sh passes it)")
    ap.add_argument("--engine-folder", required=True, help="the engine folder (debug-panel.sh passes it)")
    ap.add_argument("--cwd", default=os.getcwd(), help="the folder relative paths start from (debug-panel.sh passes it)")
    args = ap.parse_args()

    folder = os.path.abspath(os.path.join(args.cwd, os.path.expanduser(args.build_folder)))
    with SoftAgent(args.server, args.engine_folder, "debug-panel") as sa:
        try:
            # The board that fits this build, so another board connected too
            # doesn't get in the way.
            port = args.port or board_port(sa, folder)[0]
            open_panel(sa, folder, port, not args.no_open)
        except ToolFailed as e:
            sys.exit(str(e))


if __name__ == "__main__":
    main()
