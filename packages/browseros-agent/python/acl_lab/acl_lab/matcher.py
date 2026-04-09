from __future__ import annotations

import json
import logging
import os
import re
import tempfile
from dataclasses import dataclass
from difflib import SequenceMatcher
from functools import lru_cache
from pathlib import Path
from typing import Iterable
from urllib.parse import urlparse

import nltk
from nltk.corpus import stopwords
from nltk.tokenize import wordpunct_tokenize

from .types import AclRule, ElementProperties, MatchDecision, MatchFixture, RuleScore

logger = logging.getLogger(__name__)

EXACT_WEIGHT = 0.25
FUZZY_WEIGHT = 0.25
SEMANTIC_WEIGHT = 0.50
BLOCK_THRESHOLD = 0.4
NLTK_DATA_DIR = Path(tempfile.gettempdir()) / "browseros-acl_lab_nltk_data"


@dataclass(frozen=True)
class SemanticScore:
    score: float
    backend: str


@dataclass(frozen=True)
class RuleMatchInputs:
    terms: list[str]
    rule_text: str
    element_fields: list[str]
    element_text: str


# Text normalization
def split_identifier_words(value: str) -> str:
    value = re.sub(r"([a-z0-9])([A-Z])", r"\1 \2", value)
    value = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1 \2", value)
    value = re.sub(r"[_-]+", " ", value)
    return value


def normalize_text(value: str) -> str:
    value = split_identifier_words(value)
    return re.sub(r"[^a-z0-9]+", " ", value.lower()).strip()


@lru_cache(maxsize=1)
def ensure_nltk_stopwords() -> None:
    NLTK_DATA_DIR.mkdir(parents=True, exist_ok=True)
    data_dir = str(NLTK_DATA_DIR)

    if data_dir not in nltk.data.path:
        nltk.data.path.append(data_dir)

    try:
        stopwords.words("english")
    except LookupError:
        nltk.download("stopwords", download_dir=data_dir, quiet=True)
        stopwords.words("english")


@lru_cache(maxsize=1)
def get_stop_words() -> set[str]:
    ensure_nltk_stopwords()
    return set(stopwords.words("english"))


def tokenize_words(value: str) -> list[str]:
    return [
        token
        for token in wordpunct_tokenize(normalize_text(value))
        if token and token.isalnum()
    ]


def normalize_term(term: str) -> str:
    return " ".join(tokenize_words(term))


def dedupe(values: Iterable[str]) -> list[str]:
    seen: set[str] = set()
    result: list[str] = []
    for value in values:
        if not value or value in seen:
            continue
        seen.add(value)
        result.append(value)
    return result


def dedupe_text_tokens(value: str) -> str:
    return " ".join(dedupe(value.split()))


# Site matching
def site_pattern_to_regex(pattern: str) -> re.Pattern[str]:
    slash_idx = pattern.find("/")
    host_part = pattern if slash_idx == -1 else pattern[:slash_idx]
    path_part = "" if slash_idx == -1 else pattern[slash_idx:]

    def escape_and_glob(value: str, slash_wild: bool) -> str:
        value = re.escape(value)
        value = value.replace(r"\*\*", "{{GLOBSTAR}}")
        value = value.replace(r"\*", ".*" if slash_wild else r"[^./]*")
        value = value.replace(r"\?", ".")
        return value.replace("{{GLOBSTAR}}", ".*")

    host_regex = escape_and_glob(host_part, False)
    path_regex = escape_and_glob(path_part, True) if path_part else r"(?:/.*)?"
    return re.compile(rf"^{host_regex}{path_regex}$", re.IGNORECASE)


def matches_site_pattern(url: str, pattern: str) -> bool:
    if not pattern:
        return False
    if pattern == "*":
        return True

    try:
        parsed = urlparse(url)
        hostname = parsed.hostname or ""
        is_simple_domain = "*" not in pattern and "/" not in pattern
        if is_simple_domain:
            return hostname == pattern or hostname.endswith(f".{pattern}")

        full_path = f"{hostname}{parsed.path or '/'}"
        return bool(site_pattern_to_regex(pattern).match(full_path))
    except Exception:
        return False


# Rule and element feature extraction
def extract_host_terms(pattern: str) -> set[str]:
    host = pattern.split("/")[0] if "/" in pattern else pattern
    normalized = tokenize_words(host.replace("*", " "))
    return {term for term in normalized if len(term) >= 3}


def compile_rule_terms(rule: AclRule) -> list[str]:
    terms: list[str] = []

    text_match = normalize_term(rule.get("textMatch", ""))
    if text_match:
        terms.append(text_match)

    description_text = rule.get("description", "")
    description = normalize_term(description_text)
    if not description:
        return dedupe(terms)

    terms.append(description)

    host_terms = extract_host_terms(rule.get("sitePattern", ""))
    stop_words = get_stop_words()
    description_tokens = tokenize_words(description_text)
    raw_terms = [
        token
        for token in description_tokens
        if len(token) >= 3 and token not in stop_words and token not in host_terms
    ]
    terms.extend(raw_terms)

    for window in (2, 3):
        if len(raw_terms) < window:
            continue
        for start in range(len(raw_terms) - window + 1):
            terms.append(" ".join(raw_terms[start : start + window]))

    return dedupe(terms)


def build_rule_text(rule: AclRule) -> str:
    return normalize_text(
        " ".join([rule.get("textMatch", ""), rule.get("description", "")])
    )


def selector_matches_props(selector: str, props: ElementProperties) -> bool:
    tag = props.get("tagName", "").lower()
    attributes = props.get("attributes", {}) or {}
    element_id = attributes.get("id")
    classes = [value for value in attributes.get("class", "").split() if value]

    for part in (value.strip() for value in selector.split(",")):
        if not part:
            continue
        if part.startswith("#") and element_id and part == f"#{element_id}":
            return True
        if part.startswith(".") and any(part == f".{name}" for name in classes):
            return True
        match = re.match(r"^(\w+)", part)
        if match and match.group(1).lower() == tag:
            return True

    return False


def build_search_fields(props: ElementProperties) -> list[str]:
    attributes = props.get("attributes", {}) or {}
    raw_fields = [
        props.get("labelText", ""),
        props.get("ariaLabel", ""),
        props.get("textContent", ""),
        attributes.get("placeholder", ""),
        attributes.get("title", ""),
        attributes.get("name", ""),
        attributes.get("value", ""),
        attributes.get("id", ""),
        props.get("role", ""),
    ]
    return dedupe(normalize_term(field) for field in raw_fields if field)


def build_search_text(props: ElementProperties) -> str:
    return dedupe_text_tokens(" ".join(build_search_fields(props)))


def build_rule_match_inputs(rule: AclRule, props: ElementProperties) -> RuleMatchInputs:
    return RuleMatchInputs(
        terms=compile_rule_terms(rule),
        rule_text=build_rule_text(rule),
        element_fields=build_search_fields(props),
        element_text=build_search_text(props),
    )


# Similarity scoring
def phrase_windows(text: str, phrase_token_count: int) -> Iterable[str]:
    tokens = text.split()
    if not tokens:
        return []
    if phrase_token_count <= 1:
        return tokens
    if len(tokens) <= phrase_token_count:
        return [" ".join(tokens)]
    return [
        " ".join(tokens[idx : idx + phrase_token_count])
        for idx in range(len(tokens) - phrase_token_count + 1)
    ]


def exact_score(terms: list[str], fields: list[str]) -> tuple[float, list[str]]:
    matched_terms = [
        term
        for term in terms
        if any(term and field and term in field for field in fields)
    ]
    return (1.0 if matched_terms else 0.0, dedupe(matched_terms))


def fuzzy_score(terms: list[str], fields: list[str]) -> float:
    best = 0.0
    for term in terms:
        token_count = max(len(term.split()), 1)
        for field in fields:
            candidates = list(phrase_windows(field, token_count)) or [field]
            for candidate in candidates:
                best = max(best, SequenceMatcher(None, term, candidate).ratio())
    return best


@lru_cache(maxsize=1)
def load_embedding_model():
    model_name = os.environ.get(
        "ACL_LAB_EMBEDDING_MODEL",
        "BAAI/bge-small-en-v1.5",
    )
    from sentence_transformers import SentenceTransformer

    return SentenceTransformer(model_name)


def semantic_similarity(left: str, right: str) -> SemanticScore:
    if not left or not right:
        return SemanticScore(0.0, "none")

    model = load_embedding_model()
    left_vector, right_vector = model.encode([left, right], normalize_embeddings=True)
    score = float(sum(a * b for a, b in zip(left_vector, right_vector)))
    return SemanticScore(max(0.0, min(score, 1.0)), "sentence-transformers")


def weighted_score(exact: float, fuzzy: float, semantic: float) -> float:
    return EXACT_WEIGHT * exact + FUZZY_WEIGHT * fuzzy + SEMANTIC_WEIGHT * semantic


# Rule scoring
def has_content_filter(rule: AclRule) -> bool:
    return bool(rule.get("selector") or rule.get("textMatch") or rule.get("description"))


def score_selector_mismatch(rule: AclRule) -> RuleScore:
    return RuleScore(
        rule_id=rule.get("id", ""),
        blocked=False,
        confidence=0.0,
        exact_score=0.0,
        fuzzy_score=0.0,
        semantic_score=0.0,
        semantic_backend="none",
        selector_matched=False,
        site_matched=True,
        reason="selector-mismatch",
    )


def score_site_only_rule(rule: AclRule, selector_matched: bool) -> RuleScore:
    return RuleScore(
        rule_id=rule.get("id", ""),
        blocked=True,
        confidence=1.0,
        exact_score=1.0,
        fuzzy_score=1.0,
        semantic_score=1.0,
        semantic_backend="site-only",
        selector_matched=selector_matched,
        site_matched=True,
        reason="site-only-rule",
    )


def score_selector_only_rule(rule: AclRule, selector_matched: bool) -> RuleScore:
    confidence = 1.0 if selector_matched else 0.0
    return RuleScore(
        rule_id=rule.get("id", ""),
        blocked=selector_matched,
        confidence=confidence,
        exact_score=confidence,
        fuzzy_score=confidence,
        semantic_score=confidence,
        semantic_backend="selector-only",
        selector_matched=selector_matched,
        site_matched=True,
        reason="selector-only",
    )


def determine_match_reason(exact: float, confidence: float) -> str:
    if exact >= 1.0:
        return "exact-term-match"
    if confidence >= BLOCK_THRESHOLD:
        return "weighted-match"
    return "below-threshold"


def log_rule_score(result: RuleScore) -> None:
    logger.debug(
        "rule %s: %s (confidence=%.4f exact=%.4f fuzzy=%.4f semantic=%.4f [%s])",
        result.rule_id,
        result.reason,
        result.confidence,
        result.exact_score,
        result.fuzzy_score,
        result.semantic_score,
        result.semantic_backend,
    )


def score_rule(
    page_url: str,
    props: ElementProperties,
    rule: AclRule,
) -> RuleScore | None:
    if not rule.get("enabled", True):
        return None

    if not matches_site_pattern(page_url, rule.get("sitePattern", "")):
        return None

    selector = rule.get("selector")
    selector_matched = True
    if selector:
        selector_matched = selector_matches_props(selector, props)
        if not selector_matched:
            return score_selector_mismatch(rule)

    if not has_content_filter(rule):
        return score_site_only_rule(rule, selector_matched)

    match_inputs = build_rule_match_inputs(rule, props)
    if not match_inputs.terms:
        return score_selector_only_rule(rule, selector_matched)

    exact, matched_terms = exact_score(match_inputs.terms, match_inputs.element_fields)
    fuzzy = fuzzy_score(match_inputs.terms, match_inputs.element_fields)
    semantic = semantic_similarity(match_inputs.rule_text, match_inputs.element_text)
    confidence = round(weighted_score(exact, fuzzy, semantic.score), 4)

    result = RuleScore(
        rule_id=rule.get("id", ""),
        blocked=confidence >= BLOCK_THRESHOLD,
        confidence=confidence,
        exact_score=round(exact, 4),
        fuzzy_score=round(fuzzy, 4),
        semantic_score=round(semantic.score, 4),
        semantic_backend=semantic.backend,
        selector_matched=selector_matched,
        site_matched=True,
        reason=determine_match_reason(exact, confidence),
        matched_terms=matched_terms,
    )
    log_rule_score(result)
    return result


# Match decision
def score_fixture(fixture: MatchFixture) -> MatchDecision:
    tool_name = fixture.get("tool_name", "")
    page_url = fixture.get("page_url", "")
    element = fixture.get("element", {})
    rules = fixture.get("rules", [])

    candidates = [
        score
        for rule in rules
        if (score := score_rule(page_url, element, rule)) is not None
    ]
    candidates.sort(key=lambda score: score.confidence, reverse=True)

    top_candidate = candidates[0] if candidates else None
    decision = MatchDecision(
        blocked=bool(top_candidate and top_candidate.blocked),
        tool_name=tool_name,
        page_url=page_url,
        matched_rule_id=top_candidate.rule_id
        if top_candidate and top_candidate.blocked
        else None,
        confidence=top_candidate.confidence if top_candidate else 0.0,
        reason=top_candidate.reason if top_candidate else "no-matching-rules",
        candidates=candidates,
    )

    if decision.blocked:
        logger.info(
            "ACL BLOCKED tool=%s url=%s rule=%s confidence=%.4f reason=%s",
            tool_name,
            page_url,
            decision.matched_rule_id,
            decision.confidence,
            decision.reason,
        )
    else:
        logger.debug(
            "ACL ALLOWED tool=%s url=%s reason=%s",
            tool_name,
            page_url,
            decision.reason,
        )

    return decision


def load_fixture(path: str | Path) -> MatchFixture:
    return json.loads(Path(path).read_text())
