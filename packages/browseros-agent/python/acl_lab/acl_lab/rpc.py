from __future__ import annotations

import json
import logging
import os
import sys
import traceback
from typing import Any, TypedDict

from .matcher import build_search_text, compile_rule_terms, normalize_text, score_fixture

logging.basicConfig(
    stream=sys.stderr,
    level=logging.DEBUG if os.environ.get("BROWSEROS_ACL_DEBUG") else logging.INFO,
    format="[acl_lab] %(levelname)s %(name)s: %(message)s",
)
logger = logging.getLogger(__name__)


class RpcRequest(TypedDict, total=False):
    id: int
    type: str
    payload: dict[str, Any]


def write_message(message: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def handle_check_acl(request_id: int, payload: dict[str, Any]) -> None:
    tool_name = payload.get("toolName", "")
    page_url = payload.get("pageUrl", "")
    props = payload.get("element", {}) or {}
    rules = payload.get("rules", []) or []

    logger.info(
        "check_acl id=%s tool=%s url=%s rules=%s tag=%s",
        request_id,
        tool_name,
        page_url,
        len(rules),
        props.get("tagName", ""),
    )

    decision = score_fixture(
        {
            "tool_name": tool_name,
            "page_url": page_url,
            "element": props,
            "rules": rules,
        }
    )
    top_candidate = decision.candidates[0] if decision.candidates else None
    comparison_rule = None
    if top_candidate:
        comparison_rule = next(
            (rule for rule in rules if rule.get("id") == top_candidate.rule_id),
            None,
        )

    logger.info(
        (
            "decision id=%s blocked=%s rule=%s confidence=%.4f reason=%s "
            "semantic=%.4f backend=%s exact=%.4f fuzzy=%.4f"
        ),
        request_id,
        decision.blocked,
        decision.matched_rule_id,
        decision.confidence,
        decision.reason,
        top_candidate.semantic_score if top_candidate else 0.0,
        top_candidate.semantic_backend if top_candidate else "none",
        top_candidate.exact_score if top_candidate else 0.0,
        top_candidate.fuzzy_score if top_candidate else 0.0,
    )
    if comparison_rule:
        logger.info(
            (
                "comparison_strings id=%s candidate_rule=%s blocked=%s "
                "rule_text=%r compiled_terms=%r element_text=%r"
            ),
            request_id,
            comparison_rule.get("id"),
            decision.blocked,
            normalize_text(
                " ".join(
                    [
                        comparison_rule.get("textMatch", ""),
                        comparison_rule.get("description", ""),
                    ]
                )
            ),
            compile_rule_terms(comparison_rule),
            build_search_text(props),
        )
    if top_candidate and top_candidate.matched_terms:
        logger.info(
            "matched_terms id=%s rule=%s terms=%s",
            request_id,
            top_candidate.rule_id,
            ",".join(top_candidate.matched_terms[:8]),
        )

    write_message(
        {
            "id": request_id,
            "ok": True,
            "result": {
                "blocked": decision.blocked,
                "matchedRuleId": decision.matched_rule_id,
                "confidence": decision.confidence,
                "semanticScore": top_candidate.semantic_score
                if top_candidate
                else 0.0,
                "semanticBackend": top_candidate.semantic_backend
                if top_candidate
                else "none",
            },
        }
    )


def handle_request(raw_line: str) -> None:
    try:
        request: RpcRequest = json.loads(raw_line)
        request_id = int(request.get("id", 0))
        request_type = request.get("type")
        payload = request.get("payload", {}) or {}

        if request_type == "check_acl":
            handle_check_acl(request_id, payload)
            return

        logger.warning(
            "unsupported request type id=%s type=%s",
            request_id,
            request_type,
        )
        write_message(
            {
                "id": request_id,
                "ok": False,
                "error": f"Unsupported request type: {request_type}",
            }
        )
    except Exception as exc:
        logger.exception("failed to handle ACL RPC request: %s", exc)
        write_message(
            {
                "id": 0,
                "ok": False,
                "error": str(exc),
                "traceback": traceback.format_exc(),
            }
        )


def main() -> None:
    logger.info("ACL lab RPC worker started")
    for line in sys.stdin:
        raw_line = line.strip()
        if not raw_line:
            continue
        handle_request(raw_line)


if __name__ == "__main__":
    main()
