from __future__ import annotations

import argparse
import json
from pathlib import Path

from .matcher import load_fixture, score_fixture


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="acl_lab")
    subparsers = parser.add_subparsers(dest="command", required=True)

    score = subparsers.add_parser("score", help="Score fixture files")
    score.add_argument("fixtures", nargs="+", help="Fixture JSON files to score")
    score.add_argument(
        "--pretty",
        action="store_true",
        help="Pretty-print JSON output",
    )

    return parser


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()

    if args.command == "score":
        results = []
        for fixture_path in args.fixtures:
            fixture = load_fixture(fixture_path)
            decision = score_fixture(fixture)
            results.append(
                {
                    "fixture": str(Path(fixture_path)),
                    "decision": decision.to_dict(),
                }
            )

        if args.pretty:
            print(json.dumps(results, indent=2))
        else:
            print(json.dumps(results))


if __name__ == "__main__":
    main()
