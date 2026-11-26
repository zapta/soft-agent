"""Calls one soft-agent tool from the shell, with no AI agent, and prints
its result as JSON:

    python scripts/tool.py TOOL [key=value ...]

such as `python scripts/tool.py list_devices` or `python scripts/tool.py
install_driver device=usb:...`. A value is JSON when it reads as JSON (a
number, true, false, a list), else text. It uses `cargo build --release`'s
soft-agent and the scripts' engine folder (~/.logix/soft-agent/engine-data);
--server and --engine-folder change them. Exits 1 when the tool fails, with
its error.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from softagent import SoftAgent, ToolFailed, default_engine_folder, default_server  # noqa: E402


def value(text):
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return text


def main():
    ap = argparse.ArgumentParser(description="Call one soft-agent tool and print its result.")
    ap.add_argument("tool", help="the tool's name, such as list_devices")
    ap.add_argument("args", nargs="*", help="its arguments, as key=value")
    ap.add_argument("--server", default=default_server(), help="the soft-agent binary; target/release/soft-agent when left out")
    ap.add_argument("--engine-folder", default=default_engine_folder(), help="the engine folder; ~/.logix/soft-agent/engine-data when left out")
    a = ap.parse_args()
    request = {}
    for arg in a.args:
        key, eq, text = arg.partition("=")
        if not eq:
            sys.exit(f"{arg!r} isn't key=value.")
        request[key] = value(text)
    with SoftAgent(a.server, a.engine_folder, "tool-script") as sa:
        try:
            print(json.dumps(sa.tool(a.tool, request), indent=2))
        except ToolFailed as e:
            print(json.dumps(e.error, indent=2))
            sys.exit(1)


if __name__ == "__main__":
    main()
