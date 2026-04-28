#!/usr/bin/env python3
"""
Evaluation helper for WebArena-Infinity verifier scripts.

Reads JSON from stdin with app_server_url, verifier_path, and task_id.
Runs the verifier against the app server and outputs a JSON result.

Verifiers have the signature: verify(server_url: str) -> tuple[bool, str]
They fetch /api/state internally and return (passed, message).

Usage:
    echo '{"app_server_url": "http://localhost:8000", "verifier_path": "/path/to/verify.py"}' | python infinity-evaluate.py
"""

import importlib.util
import json
import sys
import traceback


def load_verifier(verifier_path: str):
    spec = importlib.util.spec_from_file_location("verifier", verifier_path)
    if spec is None or spec.loader is None:
        raise ImportError(f"Cannot load verifier from {verifier_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    try:
        data = json.loads(sys.stdin.read())
    except json.JSONDecodeError as e:
        print(json.dumps({"pass": False, "reward": 0.0, "message": f"Invalid JSON input: {e}"}))
        sys.exit(1)

    server_url = data.get("app_server_url", "")
    verifier_path = data.get("verifier_path", "")

    if not server_url or not verifier_path:
        print(json.dumps({
            "pass": False,
            "reward": 0.0,
            "message": "Missing app_server_url or verifier_path",
        }))
        sys.exit(1)

    try:
        verifier = load_verifier(verifier_path)
        fn = getattr(verifier, "verify", None)
        if not callable(fn):
            raise AttributeError(
                f"Verifier has no verify() function. "
                f"Available: {[a for a in dir(verifier) if not a.startswith('_')]}"
            )

        # Verifiers take server_url and fetch state internally
        result = fn(server_url)

        # Return is tuple[bool, str]
        if isinstance(result, tuple) and len(result) >= 2:
            passed, message = result[0], str(result[1])
        else:
            passed, message = bool(result), str(result)

    except Exception as e:
        print(json.dumps({
            "pass": False,
            "reward": 0.0,
            "message": f"Verifier error: {e}\n{traceback.format_exc()}",
        }))
        sys.exit(1)

    print(json.dumps({
        "pass": passed,
        "reward": 1.0 if passed else 0.0,
        "message": message,
    }))


if __name__ == "__main__":
    main()
