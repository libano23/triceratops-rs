#!/usr/bin/env python3
"""Check that `<triceratops> mcp` answers as an MCP server over stdin/stdout.

    TRICERATOPS_SOCKET=none python3 scripts/mcp-smoke.py target/release/triceratops

Sends initialize, notifications/initialized and tools/list, and fails unless the
Triceratops tools come back. TRICERATOPS_SOCKET points at a socket that does not
exist, so no window is started. Exits non-zero on any problem.
"""
import json
import os
import subprocess
import sys
import threading

EXPECTED = {
    "cancel", "capture_view", "find_commands", "get_state", "list_objects", "object_info", "redo", "run_commands", "undo",
    "locust_bake", "locust_components", "locust_get_graph", "locust_set_graph", "locust_set_values", "locust_solve",
}


def main() -> int:
    exe = sys.argv[1]
    env = dict(os.environ, TRICERATOPS_SOCKET=os.environ.get("TRICERATOPS_SOCKET") or "none-for-smoke-test")
    p = subprocess.Popen([exe, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env)
    timer = threading.Timer(60, p.kill)
    timer.start()
    try:
        def send(msg):
            p.stdin.write(json.dumps(msg) + "\n")
            p.stdin.flush()

        def receive():
            line = p.stdout.readline()
            if not line:
                raise SystemExit(f"{exe} mcp closed stdout without answering")
            return json.loads(line)

        send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "smoke-test", "version": "1"}}})
        init = receive()["result"]
        print("server:", init["serverInfo"]["name"], init["serverInfo"]["version"], "protocol", init["protocolVersion"])
        send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        send({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        tools = {t["name"] for t in receive()["result"]["tools"]}
        print("tools:", ", ".join(sorted(tools)))
        if tools != EXPECTED:
            print("unexpected tool list; missing:", EXPECTED - tools, "extra:", tools - EXPECTED)
            return 1
        p.stdin.close()
        p.wait(10)
        return 0
    finally:
        timer.cancel()
        if p.poll() is None:
            p.kill()


if __name__ == "__main__":
    sys.exit(main())
