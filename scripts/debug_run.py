"""Builds a design with the debugger, programs the board and opens the panel:
the whole flow from the shell, with no AI agent.

Run it through scripts/debug-run.sh, which builds soft-agent first:

    scripts/debug-run.sh PROJECT_FOLDER [--port PORT] [--no-open] [--no-upload]

or, where bash isn't at hand (Windows), run it with Python after
`cargo build --release`, which it then uses:

    python scripts/debug_run.py PROJECT_FOLDER [--port PORT] [--no-open] [--no-upload]

The project folder holds logix.json (board, top_module, and optionally
source_files, constraint_file and debug_file), its sources and its debug file;
see examples/alhambra1. The steps, each a soft-agent tool:

1. download_packages: the toolchain, the first time (a few minutes).
2. debug_build: the design with the debugger, into _build_debug.
3. program_board: the bitstream into the connected board.
4. debug_open and debug_panel: the panel in the browser, until Enter.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from softagent import SoftAgent, ToolFailed, board_port, default_engine_folder, default_server  # noqa: E402
from debug_panel import open_panel  # noqa: E402


def step(text):
    print(f"\n== {text}", flush=True)


def project(folder):
    """The project's settings from logix.json, with defaults: every .v and
    .sv file except testbenches, the one constraint file, debug.toml."""
    path = os.path.join(folder, "logix.json")
    try:
        with open(path) as f:
            cfg = json.load(f)
    except OSError:
        sys.exit(f"No logix.json in {folder}: is it a project folder?")
    except json.JSONDecodeError as e:
        sys.exit(f"{path} isn't valid JSON: {e}")
    for key in ("board", "top_module"):
        if key not in cfg:
            sys.exit(f"{path} has no {key}.")
    names = sorted(os.listdir(folder))
    cfg.setdefault("source_files", [
        n for n in names
        if n.endswith((".v", ".sv")) and not n.removesuffix(".sv").removesuffix(".v").endswith("_tb")
    ])
    if "constraint_file" not in cfg:
        pins = [n for n in names if n.endswith((".pcf", ".lpf", ".cst", ".xdc"))]
        if len(pins) != 1:
            sys.exit(f"{folder} has {len(pins)} constraint files: name one as constraint_file in logix.json.")
        cfg["constraint_file"] = pins[0]
    cfg.setdefault("debug_file", "debug.toml")
    return cfg


def main():
    ap = argparse.ArgumentParser(description="Build with the debugger, upload, and open the debug panel.")
    ap.add_argument("project", help="the project folder, such as examples/alhambra1")
    ap.add_argument("--port", help="the board's serial port; found by list_devices when left out")
    ap.add_argument("--no-open", action="store_true", help="print the panel's address without opening it")
    ap.add_argument("--no-upload", action="store_true", help="only build: don't program the board or open the panel")
    ap.add_argument("--server", default=default_server(), help="the soft-agent binary; target/release/soft-agent when left out")
    ap.add_argument("--engine-folder", default=default_engine_folder(), help="the engine folder; ~/.logix/soft-agent/engine-data when left out")
    ap.add_argument("--cwd", default=os.getcwd(), help="the folder relative paths start from (debug-run.sh passes it)")
    args = ap.parse_args()

    folder = os.path.abspath(os.path.join(args.cwd, os.path.expanduser(args.project)))
    cfg = project(folder)
    build_folder = os.path.join(folder, "_build_debug")

    with SoftAgent(args.server, args.engine_folder, "debug-run") as sa:
        try:
            step("Checking the toolchain")
            check = sa.tool("download_packages", {"check_only": True})
            if check.get("to_download"):
                for line in check.get("summary", []):
                    print(line)
                print("Downloading it now, once: this takes a few minutes...", flush=True)
                for line in sa.tool("download_packages", {}).get("summary", []):
                    print(line)
            else:
                print("Ready.")

            step(f"Building {cfg['top_module']} for the {cfg['board']} with the debugger")
            build = sa.tool("debug_build", {
                "board": cfg["board"],
                **({"board_variant": cfg["board_variant"]} if "board_variant" in cfg else {}),
                "top_module": cfg["top_module"],
                "source_folder": folder,
                "source_files": cfg["source_files"],
                "constraint_file": cfg["constraint_file"],
                "debug_file": cfg["debug_file"],
                "build_folder": build_folder,
            })
            print(build["build"].get("report", "").strip())
            print(build["debugger"]["report"])
            print(f"Build ID {build['build_id']}: drives "
                  + ", ".join(d["name"] for d in build["drive"])
                  + "; watches " + ", ".join(w["name"] for w in build["watch"]) + ".")
            if args.no_upload:
                return

            step("Programming the board")
            port, device = board_port(sa, build_folder)
            port = args.port or port
            sa.tool("program_board", {"build_folder": build_folder, "device": device})
            print("Done.")

            # The FPGA starts the design; from flash that can take seconds.
            step("Opening the debug panel")
            open_panel(sa, build_folder, port, not args.no_open, wait=15)
        except ToolFailed as e:
            err = e.error
            print(f"\n{e.tool} failed: {err.get('message', err)}", file=sys.stderr)
            for d in err.get("diagnostics", [])[:10]:
                where = f"{d.get('file', '?')}:{d.get('line', '?')}"
                print(f"  {where}: {d.get('text', '')}", file=sys.stderr)
            action = (err.get("action") or {}).get("description")
            if action:
                print(f"  {action}", file=sys.stderr)
            sys.exit(1)


if __name__ == "__main__":
    main()
