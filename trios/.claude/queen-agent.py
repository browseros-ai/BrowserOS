#!/usr/bin/env python3
"""
Queen BrowserOS - Autonomous Agent Brain
Uses BrowserOS MCP server (127.0.0.1:9105)
"""

import json
import urllib.request
import os
from datetime import datetime

MCP_URL = "http://127.0.0.1:9105/mcp"
REPO = "/Users/playra/BrowserOS-full/trios"

def mcp_call(tool_name, arguments):
    req = urllib.request.Request(
        MCP_URL,
        data=json.dumps({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": tool_name, "arguments": arguments}
        }).encode(),
        headers={"Content-Type": "application/json"}
    )
    try:
        resp = urllib.request.urlopen(req, timeout=30)
        return json.loads(resp.read())
    except Exception as e:
        return {"error": str(e)}

def log(msg):
    ts = datetime.now().strftime("%Y-%m-%d_%H:%M:%S")
    line = f"[{ts}] {msg}"
    print(line)
    with open(f"{REPO}/.trinity/queen.log", "a") as f:
        f.write(line + "\n")

class QueenBrain:
    def __init__(self):
        self.memory = {}
        self.load_state()

    def load_state(self):
        p = f"{REPO}/.trinity/queen_state.json"
        if os.path.exists(p):
            with open(p) as f:
                self.memory = json.load(f)

    def save_state(self):
        with open(f"{REPO}/.trinity/queen_state.json", "w") as f:
            json.dump(self.memory, f, indent=2)

    def perceive(self):
        log("PERCEIVE: reading environment...")
        h = mcp_call("shell_execute", {"command": "curl -s http://127.0.0.1:9105/health"})
        b = mcp_call("shell_execute", {"command": f"test -f {REPO}/trios_app && echo BINARY_OK || echo NO_BINARY"})
        r = mcp_call("shell_execute", {"command": f"find {REPO}/rings -name *.swift | wc -l"})
        s = mcp_call("shell_execute", {"command": f"ls {REPO}/.claude/skills/*/SKILL.md 2>/dev/null | wc -l"})
        a = mcp_call("shell_execute", {"command": f"ls {REPO}/.claude/agents/*.md 2>/dev/null | wc -l"})
        self.memory["perception"] = {"ts": datetime.now().isoformat(), "health": h, "build": b, "rings": r, "skills": s, "agents": a}
        return self.memory["perception"]

    def think(self, p):
        log("THINK: analyzing...")
        actions = []
        if "NO_BINARY" in str(p.get("build", "")):
            actions.append({"tool": "shell_execute", "args": {"command": f"cd {REPO} && ./build.sh"}, "reason": "No binary"})
        git = mcp_call("shell_execute", {"command": f"cd {REPO} && git status --porcelain | wc -l"})
        dirty = 0
        try:
            c = git["result"]["content"][0]["text"]
            dirty = int(c.strip())
        except:
            dirty = 0
        if dirty > 0:
            actions.append({"tool": "shell_execute", "args": {"command": f"cd {REPO} && git add -A && git commit -m chore: auto-commit || true"}, "reason": f"{dirty} dirty"})
        actions.append({"tool": "fs_write", "args": {"path": f"{REPO}/.trinity/queen_alive.txt", "content": f"Queen alive at {datetime.now()}"}, "reason": "Alive marker"})
        self.memory["thoughts"] = actions
        return actions

    def act(self, actions):
        log(f"ACT: {len(actions)} actions")
        for a in actions:
            log(f"  - {a['reason']}")
            mcp_call(a["tool"], a["args"])
        self.memory["actions"] = actions

    def reflect(self):
        log("REFLECT: saving...")
        self.save_state()
        log("Done\n")

    def run(self):
        log("==== QUEEN AWAKES ====")
        p = self.perceive()
        t = self.think(p)
        self.act(t)
        self.reflect()

if __name__ == "__main__":
    QueenBrain().run()
