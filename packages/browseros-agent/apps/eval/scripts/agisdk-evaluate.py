#!/usr/bin/env python3
"""
AGI SDK evaluation helper for BrowserOS eval framework.

Reads JSON from stdin with task_id and env_state, runs the agisdk
evaluator, and outputs the result as JSON to stdout.

Input format:
    {"task_id": "dashdish-1", "env_state": {...}, "model_response": ""}

Output format:
    {"reward": 0.0, "pass": false, "message": "...", "per_criterion": [...]}
"""

import json
import sys


def main():
    data = json.loads(sys.stdin.read())
    task_id = data["task_id"]
    env_state = data["env_state"]
    model_response = data.get("model_response", "")

    try:
        from agisdk.REAL.browsergym.webclones.evaluate import WebCloneEvaluator
        from agisdk.REAL.browsergym.webclones.task_config import TaskConfig
    except ImportError:
        print(
            json.dumps(
                {
                    "reward": 0,
                    "pass": False,
                    "message": "agisdk package not installed. Run: pip install agisdk",
                    "per_criterion": [],
                }
            )
        )
        sys.exit(0)

    try:
        # Redirect stdout to stderr during evaluation — agisdk's rich logger
        # prints directly to stdout, which would corrupt our JSON output
        real_stdout = sys.stdout
        sys.stdout = sys.stderr

        tc = TaskConfig(task_id)
        evaluator = WebCloneEvaluator(tc)
        reward_val, _done, message, info = evaluator.evaluate(
            env_state=env_state, model_response=model_response
        )

        sys.stdout = real_stdout

        reward_val = float(reward_val) if reward_val is not None else 0.0
        results = info.get("results", [])
        per_criterion = [
            {"passed": r[0], "detail": str(r[1]) if len(r) > 1 else ""}
            for r in results
        ]

        print(
            json.dumps(
                {
                    "reward": reward_val,
                    "pass": reward_val == 1.0,
                    "message": str(message),
                    "per_criterion": per_criterion,
                }
            )
        )

    except Exception as e:
        sys.stdout = real_stdout if "real_stdout" in dir() else sys.__stdout__
        print(
            json.dumps(
                {
                    "reward": 0,
                    "pass": False,
                    "message": f"Evaluation error: {str(e)}",
                    "per_criterion": [],
                }
            )
        )


if __name__ == "__main__":
    main()
