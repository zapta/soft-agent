"""Opens the debug panel for a debug build on the board, from the shell.

Run it through scripts/debug-panel.sh, which builds soft-agent first:

    scripts/debug-panel.sh BUILD_FOLDER [--port PORT] [--no-open]

or with Python after `cargo build --release` (`python
scripts/debug_panel.py BUILD_FOLDER ...`), where bash isn't at hand.

It starts soft-agent, opens a debug session (open_debug) and the panel
(open_debug_panel), and waits; Enter or Ctrl-C closes the session, which releases
every override and frees the port. Without --port it uses the serial port
of the one connected board that fits the build.
"""

import argparse
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from softagent import SoftAgent, ToolFailed, board_port, default_engine_folder, default_server  # noqa: E402


def open_session(sa, build_folder, port, wait=0, device=None):
    """open_debug, trying again for up to `wait` seconds while the debugger
    doesn't answer, as while an FPGA loads its design from flash. With
    `device` (its list_devices id), the session finds the board again
    after an unplug, even under another port name."""
    deadline = time.monotonic() + wait
    while True:
        try:
            args = {"build_folder": build_folder, "port": port}
            if device:
                args["device"] = device
            return sa.tool("open_debug", args)
        except ToolFailed as e:
            if "didn't answer" not in e.error.get("message", "") or time.monotonic() > deadline:
                raise
            time.sleep(1)


def open_panel(sa, build_folder, port, open_browser=True, wait=0, device=None):
    """Opens a session and the panel, waits for Enter or Ctrl-C, then
    closes the session. `wait` and `device` are as for open_session."""
    s = open_session(sa, build_folder, port, wait, device)
    print(f"Session {s['session']} on {port}, build {s['build_id']}")
    panel = sa.tool("open_debug_panel", {"session": s["session"], "open": open_browser})
    print(f"Panel: {panel['url']}")
    try:
        input("Press Enter to close the session (or Ctrl-C)... ")
    except (KeyboardInterrupt, EOFError):
        print()
    finally:
        released = sa.tool("close_debug", {"session": s["session"]})["released"]
        print("Closed; overrides released." if released else "Closed; couldn't release the overrides.")


def main():
    ap = argparse.ArgumentParser(description="Open the soft-agent debug panel.")
    ap.add_argument("build_folder", help="the debug build's folder, such as ~/my-design/_build_debug")
    ap.add_argument("--port", help="the board's serial port; found by list_devices when left out")
    ap.add_argument("--no-open", action="store_true", help="print the panel's address without opening it")
    ap.add_argument("--server", default=default_server(), help="the soft-agent binary; target/release/soft-agent when left out")
    ap.add_argument("--engine-folder", default=default_engine_folder(), help="the engine folder; ~/.logix/soft-agent/engine-data when left out")
    ap.add_argument("--cwd", default=os.getcwd(), help="the folder relative paths start from (debug-panel.sh passes it)")
    args = ap.parse_args()

    folder = os.path.abspath(os.path.join(args.cwd, os.path.expanduser(args.build_folder)))
    with SoftAgent(args.server, args.engine_folder, "debug-panel") as sa:
        try:
            # The board that fits this build, so another board connected too
            # doesn't get in the way.
            port, device = (args.port, None) if args.port else board_port(sa, folder)
            open_panel(sa, folder, port, not args.no_open, device=device)
        except ToolFailed as e:
            sys.exit(str(e))


if __name__ == "__main__":
    main()
