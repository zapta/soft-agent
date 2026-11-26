"""Downloads the FPGA toolchain and board catalog soft-agent uses, if they
aren't there yet, through its download_packages tool. install-deps.sh runs
it, so that the first build doesn't have to.

    python3 scripts/download_toolchain.py --server BIN --engine-folder DIR
"""

import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from softagent import SoftAgent, ToolFailed, default_engine_folder, default_server  # noqa: E402


def main():
    ap = argparse.ArgumentParser(description="Download soft-agent's FPGA toolchain.")
    ap.add_argument("--server", default=default_server(), help="the soft-agent binary; target/release/soft-agent when left out")
    ap.add_argument("--engine-folder", default=default_engine_folder(), help="the engine folder; ~/.logix/soft-agent/engine-data when left out")
    args = ap.parse_args()

    with SoftAgent(args.server, args.engine_folder, "download-toolchain") as sa:
        try:
            check = sa.tool("download_packages", {"check_only": True})
            if not check.get("to_download"):
                print("The FPGA toolchain is ready.")
                return
            for line in check.get("summary", []):
                print(line)
            print("Downloading it now: this takes a few minutes...", flush=True)
            for line in sa.tool("download_packages", {}).get("summary", []):
                print(line)
        except ToolFailed as e:
            sys.exit(f"The download failed: {e.error.get('message', e)}")


if __name__ == "__main__":
    main()
