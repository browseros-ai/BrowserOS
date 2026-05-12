#!/usr/bin/env python3
"""
Build JSONL dataset for AGI SDK / REAL Bench evaluation.

Reads task definitions from the agisdk package, filters to feasible
action-only tasks (excludes llm_boolean evaluators), and outputs JSONL
to stdout in the BrowserOS eval framework format.

Usage:
    python scripts/build-agisdk-dataset.py > data/agisdk-real.jsonl
"""

import json
import sys


def has_llm_eval(task: dict) -> bool:
    return any(e.get("type") == "llm_boolean" for e in task.get("evals", []))


def main():
    try:
        from agisdk.REAL.tasks import all_tasks
    except ImportError:
        print(
            "Error: agisdk package not installed. Run: pip install agisdk",
            file=sys.stderr,
        )
        sys.exit(1)

    count = 0
    skipped_infeasible = 0
    skipped_llm = 0

    for task in all_tasks:
        if not task.get("possible", True):
            skipped_infeasible += 1
            continue

        if has_llm_eval(task):
            skipped_llm += 1
            continue

        task_id = task["id"]
        website = task.get("website", {})
        goal = task.get("goal", "")
        start_url = website.get("url", "")

        if not start_url or not goal:
            print(f"Warning: Skipping {task_id} — missing url or goal", file=sys.stderr)
            continue

        entry = {
            "query_id": f"agisdk-{task_id}",
            "dataset": "agisdk-real",
            "query": goal,
            "graders": ["agisdk_state_diff"],
            "start_url": start_url,
            "metadata": {
                "original_task_id": task_id,
                "website": website.get("name", ""),
                "category": "agisdk-real",
                "additional": {
                    "agisdk_task_id": task_id,
                    "challenge_type": task.get("challengeType", "action"),
                    "difficulty": task.get("difficulty", "unknown"),
                    "similar_to": website.get("similarTo", ""),
                },
            },
        }

        print(json.dumps(entry))
        count += 1

    print(
        f"Generated {count} tasks (skipped {skipped_infeasible} infeasible, "
        f"{skipped_llm} llm_boolean)",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
