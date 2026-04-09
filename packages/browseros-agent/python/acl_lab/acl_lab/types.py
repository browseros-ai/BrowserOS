from __future__ import annotations

from dataclasses import asdict, dataclass, field
from typing import Any, TypedDict


class AclRule(TypedDict, total=False):
    id: str
    sitePattern: str
    selector: str
    textMatch: str
    description: str
    enabled: bool


class ElementProperties(TypedDict, total=False):
    tagName: str
    textContent: str
    attributes: dict[str, str]
    labelText: str
    ariaLabel: str
    role: str


class MatchFixture(TypedDict, total=False):
    tool_name: str
    page_url: str
    element: ElementProperties
    rules: list[AclRule]


@dataclass
class RuleScore:
    rule_id: str
    blocked: bool
    confidence: float
    exact_score: float
    fuzzy_score: float
    semantic_score: float
    semantic_backend: str
    selector_matched: bool
    site_matched: bool
    reason: str
    matched_terms: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass
class MatchDecision:
    blocked: bool
    tool_name: str
    page_url: str
    matched_rule_id: str | None
    confidence: float
    reason: str
    candidates: list[RuleScore]

    def to_dict(self) -> dict[str, Any]:
        data = asdict(self)
        data["candidates"] = [candidate.to_dict() for candidate in self.candidates]
        return data
