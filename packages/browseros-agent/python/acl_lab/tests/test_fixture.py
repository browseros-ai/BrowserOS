from __future__ import annotations

import json
from pathlib import Path

from acl_lab.matcher import score_fixture


def test_submit_button_fixture_blocks_checkout() -> None:
    fixture_path = Path(__file__).resolve().parent.parent / "fixtures" / "submit_button.json"
    fixture = json.loads(fixture_path.read_text())

    decision = score_fixture(fixture)
    print(json.dumps(decision.to_dict(), indent=2))

    assert decision.blocked is True
    assert decision.matched_rule_id == "checkout-submit"
    assert decision.reason == "exact-term-match"
    assert decision.candidates[0].semantic_backend == "sentence-transformers"
