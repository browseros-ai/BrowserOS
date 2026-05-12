#!/usr/bin/env python3
"""
Dataset generator for WebArena-Infinity benchmark.

Reads real-tasks.json from each app directory and outputs JSONL
in the eval framework's TaskSchema format.

Usage:
    python build-infinity-dataset.py --apps-dir /path/to/webarena-infinity/apps
    python build-infinity-dataset.py --apps-dir /path/to/apps --apps gmail linear --difficulty medium
"""

import argparse
import json
import os
import sys


def load_tasks(app_dir: str, app_name: str) -> list[dict]:
    tasks_file = os.path.join(app_dir, "real-tasks.json")
    if not os.path.exists(tasks_file):
        print(f"Warning: No real-tasks.json found in {app_dir}", file=sys.stderr)
        return []
    with open(tasks_file) as f:
        return json.load(f)


def build_task_entry(
    app_name: str,
    task: dict,
    base_port: int,
) -> dict:
    task_id = task.get("id", task.get("task_id", "unknown"))
    difficulty = task.get("difficulty", "unknown")
    query = task.get("query", task.get("instruction", task.get("task", "")))
    verifier_path = task.get(
        "verify",
        task.get("verifier_path", f"real-tasks/{task_id}.py"),
    )

    return {
        "query_id": f"infinity-{app_name}-{task_id}",
        "dataset": "webarena-infinity",
        "query": query,
        "graders": ["infinity_state"],
        "start_url": f"http://localhost:{base_port}",
        "setup_script": f"POST http://localhost:{base_port}/api/reset",
        "metadata": {
            "original_task_id": f"{app_name}-{task_id}",
            "website": app_name,
            "category": "webarena-infinity",
            "additional": {
                "app_name": app_name,
                "difficulty": difficulty,
                "verifier_path": verifier_path,
                "app_port": base_port,
            },
        },
    }


def main():
    parser = argparse.ArgumentParser(
        description="Generate JSONL dataset from WebArena-Infinity apps"
    )
    parser.add_argument(
        "--apps-dir",
        required=True,
        help="Path to webarena-infinity/apps/ directory",
    )
    parser.add_argument(
        "--apps",
        nargs="*",
        default=None,
        help="Filter to specific app names (default: all)",
    )
    parser.add_argument(
        "--difficulty",
        choices=["easy", "medium", "hard"],
        default=None,
        help="Filter by difficulty tier",
    )
    parser.add_argument(
        "--base-port",
        type=int,
        default=8000,
        help="Starting port number for apps (default: 8000)",
    )
    args = parser.parse_args()

    if not os.path.isdir(args.apps_dir):
        print(f"Error: {args.apps_dir} is not a directory", file=sys.stderr)
        sys.exit(1)

    app_dirs = sorted(os.listdir(args.apps_dir))
    if args.apps:
        app_dirs = [d for d in app_dirs if d in args.apps]

    port = args.base_port
    for app_name in app_dirs:
        app_path = os.path.join(args.apps_dir, app_name)
        if not os.path.isdir(app_path):
            continue

        tasks = load_tasks(app_path, app_name)
        for task in tasks:
            difficulty = task.get("difficulty", "unknown")
            if args.difficulty and difficulty != args.difficulty:
                continue

            entry = build_task_entry(app_name, task, port)
            print(json.dumps(entry))

        port += 1


if __name__ == "__main__":
    main()
