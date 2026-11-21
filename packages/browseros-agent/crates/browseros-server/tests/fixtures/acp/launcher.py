#!/usr/bin/env python3
"""Replay sanitized acpx stdout and record what the server sent at process boundaries."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

fixtures = Path(__file__).resolve().parent
args = sys.argv[1:]

def send(message):
    print(json.dumps(message, separators=(",", ":")), flush=True)

def record(**fields):
    with (fixtures / "calls.ndjson").open("a") as log:
        log.write(json.dumps(fields) + "\n")

if "--agent" not in args:
    # Direct ACP probe. Replies also include agent-originated traffic with unrelated ids.
    for line in sys.stdin:
        request = json.loads(line)
        if request.get("method") == "initialize":
            send({"jsonrpc":"2.0", "method":"fixture/update", "params":{}})
            result = {"protocolVersion":1, "agentInfo":{"name":"fixture-adapter"}}
        elif "claude-agent-acp@" in " ".join(args):
            send({"jsonrpc":"2.0", "id":request["id"], "error":{"code":-32000}})
            break
        else:
            result = {"sessionId":"probe", "models":{
                "currentModelId":"fixture[high]",
                "availableModels":[
                    {"modelId":"fixture[low]", "name":"Fixture (low)"},
                    {"modelId":"fixture[high]", "name":"Fixture (high)"},
                ],
            }}
        send({"jsonrpc":"2.0", "id":request["id"], "result":result})
    sys.exit()

assert "acpx@0.19.3" in args
command = args[args.index("--agent")+1]
config = json.loads(Path(args[args.index("--mcp-config")+1]).read_text())
home = Path(os.environ["HOME"])
after_globals = args[args.index("--suppress-reads")+1:]
if after_globals[0] == "--append-system-prompt":
    after_globals = after_globals[2:]
if after_globals[0].startswith("--model="):
    after_globals = after_globals[1:]
operation = after_globals[0]
record(operation=operation, args=after_globals, cwd=args[args.index("--cwd")+1],
       adapter=command, mcp=config, pid=os.getpid())
if operation == "sessions":
    action = after_globals[1]
    marker = home / "record"
    if action == "ensure":
        send({"action":"session_ensured", "created":not marker.exists()})
        marker.touch()
    elif action == "close":
        if (home / "stubborn-owner").exists():
            (home / "close-attempted").touch()
            time.sleep(60)
        owner = home / "owner.pid"
        if owner.exists():
            os.kill(int(owner.read_text()), signal.SIGTERM)
        if (home / "fail-close").exists():
            (home / "fail-close").unlink()
            send({"jsonrpc":"2.0","id":None,"error":{"code":-32603}})
            sys.exit(1)
        marker.unlink(missing_ok=True)
        send({"action":"session_closed"})
elif operation in ("set-mode", "set"):
    if after_globals[-1] == "unavailable-model":
        send({"jsonrpc":"2.0","id":None,"error":{"code":-32602}})
        sys.exit(1)
    send({"action":"config_set"})
elif operation == "cancel":
    (home / "cancelled").touch()
    send({"action":"cancel_result", "cancelled":True})
elif operation == "prompt":
    prompt = sys.stdin.read()
    record(prompt=prompt)
    if "spawn a fixture owner" in prompt:
        # Like acpx's queue owner, this child survives the CLI and its process group.
        owner = subprocess.Popen(["sleep", "60"], start_new_session=True,
                                 stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL)
        (home / "owner.pid").write_text(str(owner.pid))
        if "stubborn" in prompt:
            (home / "stubborn-owner").touch()
    fixture = "auth" if "claude-agent-acp@" in command else (
        "cancel" if "cancel this turn" in prompt else "success"
    )
    for line in (fixtures / (fixture + ".ndjson")).read_text().splitlines():
        message = json.loads(line)
        reply = message.get("result", {})
        if reply.get("stopReason") == "cancelled" and fixture == "cancel":
            while not (home / "cancelled").exists():
                time.sleep(.01)
            if "lose the reply" in prompt:
                sys.exit()
        if reply.get("stopReason") == "end_turn":
            if "refuse this turn" in prompt:
                reply["stopReason"] = "refusal"
            if "fail browser tools" in prompt:
                for suffix, raw in [
                    ("mcp-error", {"result": {"content": [
                        {"type": "text", "text": "[UNTRUSTED_PAGE_CONTENT nonce=fixture origin=run] Data follows.\nReal browser failure.\n"
                         + "é" * 40000 + "\n[END_UNTRUSTED_PAGE_CONTENT nonce=fixture]"},
                    ]}, "error": None}),
                    ("command-error", {"formatted_output": "Fixture command failure.", "exit_code": 2}),
                    ("no-detail", None),
                ]:
                    send({"jsonrpc":"2.0", "method":"session/update", "params":{
                        "sessionId":"fixture-session", "update":{
                            "sessionUpdate":"tool_call", "toolCallId":suffix,
                            "title":"Command" if suffix == "command-error" else "mcp.browseros.run",
                            "status":"failed", "rawOutput":raw,
                        },
                    }})
        send(message)
        if "lose the stream" in prompt and message.get("method") == "session/prompt":
            (home / "fail-close").touch()
            print("broken ACP stdout", flush=True)
            sys.exit(1)
