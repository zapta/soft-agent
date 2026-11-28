"""Checks the debugger on a connected iCESugar v1.5, with no AI agent: builds
examples/icesugar-1-5 with the debugger, programs the board, then reads,
writes, releases, holds and pulses its nets through the session tools, and
times reads and writes. Prints PASS or FAIL for each check; exits 1 if any
failed.

    python scripts/debug_check.py [--port PORT]

after `cargo build --release`. Other projects need the same drive and watch
names as examples/icesugar-1-5's debug.toml (--project).
"""

import argparse
import os
import platform
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from softagent import SoftAgent, ToolFailed, board_port, default_engine_folder, default_server  # noqa: E402
from debug_run import project  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
failed = []


def check(ok, what):
    print(("PASS  " if ok else "FAIL  ") + what, flush=True)
    if not ok:
        failed.append(what)


def main():
    ap = argparse.ArgumentParser(description="Check the debugger on a connected iCESugar v1.5.")
    ap.add_argument("--project", default=os.path.join(REPO, "examples", "icesugar-1-5"))
    ap.add_argument("--port", help="the board's serial port; found by list_devices when left out")
    ap.add_argument("--server", default=default_server(), help="the soft-agent binary; target/release/soft-agent when left out")
    ap.add_argument("--engine-folder", default=default_engine_folder(), help="the engine folder; ~/.logix/soft-agent/engine-data when left out")
    a = ap.parse_args()
    folder = os.path.abspath(a.project)
    cfg = project(folder)
    bf = os.path.join(folder, "_build_debug")
    print(f"{platform.system()} {platform.release()} ({platform.machine()}), Python {platform.python_version()}")

    with SoftAgent(a.server, a.engine_folder, "debug-check") as sa:
        try:
            info = sa.tool("get_soft_agent_info", {})
            print(f"soft-agent {info['extension_version']}, engine {info['engine_version']} ({info['engine_commit']})")
            if sa.tool("download_packages", {"check_only": True}).get("to_download"):
                sys.exit("The FPGA tools aren't downloaded yet: run `python scripts/tool.py download_packages` first.")

            print("\n== Building with the debugger (a minute or so)")
            b = sa.tool("debug_build", {
                "board": cfg["board"], "top_module": cfg["top_module"], "source_folder": folder,
                "source_files": cfg["source_files"], "constraint_file": cfg["constraint_file"],
                "debug_file": cfg["debug_file"], "build_folder": bf})
            print(b["build"].get("report", "").strip())
            print(b["debugger"]["report"])
            check(True, f"debug build {b['build_id']}")

            print("\n== Programming the board")
            port, device = board_port(sa, bf, a.port)
            print(f"device {device}\nserial port {port}")
            sa.tool("program_board", {"build_folder": bf, "device": device})
            check(True, "program_board")

            print("\n== The session")
            for i in range(30):
                try:
                    o = sa.tool("open_debug", {"build_folder": bf, "port": port, "device": device})
                    break
                except ToolFailed:
                    if i == 29:
                        raise
                    time.sleep(0.5)
            sid = o["session"]
            check(o["build_id"] == b["build_id"], f"open_debug: the board runs build {o['build_id']}")

            def read():
                return {v["name"]: v for v in sa.tool("read_debug", {"session": sid})["values"]}

            def write(**kw):
                return sa.tool("write_debug", {"session": sid, **kw})

            r1 = read()
            time.sleep(0.3)
            r2 = read()
            check(r1["timer"]["value"] != r2["timer"]["value"], f"the timer runs: {r1['timer']['value']} then {r2['timer']['value']}")
            check(r2["step_enabled"]["value"] == 1, "step_enabled is the design's 1")

            w = write(name="pattern_in", value="0xA5")
            check(w["driven"] and w["hex"] == "0xa5", f"drive pattern_in 0xA5: driven {w['driven']}, holds {w.get('hex')}")
            check(read()["pattern"]["value"] == 0xA5, "pattern reads the driven 0xA5")
            check(not write(name="pattern_in", release=True)["driven"], "release pattern_in")

            check(write(name="enable_step", value=0)["driven"], "drive enable_step 0")
            p1 = read()["pattern"]["value"]
            time.sleep(0.6)
            p2 = read()["pattern"]["value"]
            check(p1 == p2, f"the pattern holds with enable_step 0: {p1:#04x} {p2:#04x}")
            w = write(name="enable_step", pulse=True)
            p3 = read()["pattern"]["value"]
            want = ((p2 << 1) | (p2 >> 7)) & 0xFF
            check(p3 == want and w["driven"], f"one pulse steps once: {p2:#04x} to {p3:#04x} (want {want:#04x})")
            time.sleep(0.6)
            check(read()["pattern"]["value"] == p3, "and the pattern holds after it")
            write(name="enable_step", release=True)
            time.sleep(0.6)
            check(read()["pattern"]["value"] != p3, "the pattern runs again after release")

            w = write(name="colors", value="0b010")
            check(w["driven"] and read()["lit"]["value"] == 2, "colors 010: only green lit")
            write(name="colors", release=True)

            print("\n== Speed")
            n, bad = 300, 0
            t = time.perf_counter()
            for _ in range(n):
                if read()["step_enabled"]["value"] != 1:
                    bad += 1
            dt = (time.perf_counter() - t) / n
            check(bad == 0, f"{n} reads, {bad} implausible: {dt * 1000:.2f} ms a read ({1 / dt:.0f} a second)")
            t = time.perf_counter()
            for k in range(100):
                write(name="pattern_in", value=k & 0xFF)
            dt = (time.perf_counter() - t) / 100
            check(True, f"100 writes: {dt * 1000:.2f} ms a write")

            check(sa.tool("close_debug", {"session": sid})["released"], "close_debug releases everything")
        except ToolFailed as e:
            err = e.error
            print(f"\n{e.tool} failed: {err.get('message', err)}")
            action = (err.get("action") or {}).get("description")
            if action:
                print(f"  {action}")
            failed.append(e.tool)

    print("\nALL PASSED" if not failed else f"\n{len(failed)} FAILED: " + "; ".join(failed))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
