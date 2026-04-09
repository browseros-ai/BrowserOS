from __future__ import annotations

import re
from typing import TypedDict
from urllib.parse import urlparse


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


STOP_WORDS = {
    "a",
    "an",
    "and",
    "any",
    "avoid",
    "be",
    "block",
    "browseros",
    "button",
    "buttons",
    "can",
    "do",
    "from",
    "for",
    "let",
    "me",
    "never",
    "not",
    "of",
    "on",
    "or",
    "prevent",
    "should",
    "stop",
    "the",
    "this",
    "to",
}

INTENT_EXPANSIONS = [
    {
        "triggers": ["pay", "payment", "payments", "checkout", "purchase", "buy"],
        "terms": [
            "pay",
            "payment",
            "payments",
            "checkout",
            "proceed to checkout",
            "continue to checkout",
            "place order",
            "place your order",
            "submit order",
            "buy now",
            "purchase",
        ],
    },
    {
        "triggers": ["send", "email", "mail", "message"],
        "terms": [
            "send",
            "send email",
            "send message",
            "compose",
            "new message",
            "send now",
        ],
    },
    {
        "triggers": ["delete", "remove", "trash"],
        "terms": ["delete", "remove", "trash", "confirm delete"],
    },
    {
        "triggers": ["submit", "save", "confirm", "approve"],
        "terms": ["submit", "save", "confirm", "approve"],
    },
]


def normalize_text(value: str) -> str:
    return re.sub(r"[^a-z0-9]+", " ", value.lower()).strip()


def dedupe(values: list[str]) -> list[str]:
    return list(dict.fromkeys(value for value in values if value))


def site_pattern_to_regex(pattern: str) -> re.Pattern[str]:
    slash_idx = pattern.find("/")
    host_part = pattern if slash_idx == -1 else pattern[:slash_idx]
    path_part = "" if slash_idx == -1 else pattern[slash_idx:]

    def escape_and_glob(value: str, slash_wild: bool) -> str:
        value = re.sub(r"([.+^${}()|[\]\\])", r"\\\1", value)
        value = value.replace("**", "{{GLOBSTAR}}")
        value = value.replace("*", ".*" if slash_wild else r"[^./]*")
        value = value.replace("?", ".")
        return value.replace("{{GLOBSTAR}}", ".*")

    host_regex = escape_and_glob(host_part, False)
    path_regex = escape_and_glob(path_part, True) if path_part else r"(?:/.*)?"
    return re.compile(rf"^{host_regex}{path_regex}$", re.IGNORECASE)


def extract_host_terms(pattern: str) -> set[str]:
    host = pattern.split("/")[0] if "/" in pattern else pattern
    normalized = normalize_text(host.replace("*", " "))
    return {term.strip() for term in normalized.split() if len(term.strip()) >= 3}


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

        full_path = hostname + (parsed.path or "/")
        return bool(site_pattern_to_regex(pattern).match(full_path))
    except Exception:
        return False


def compile_acl_terms(rule: AclRule) -> list[str]:
    terms: list[str] = []

    text_match = rule.get("textMatch")
    if text_match:
        normalized = normalize_text(text_match)
        if normalized:
            terms.append(normalized)

    host_terms = extract_host_terms(rule.get("sitePattern", ""))
    intent_text = normalize_text(rule.get("description", ""))

    if intent_text:
        raw_terms = [
            term.strip()
            for term in intent_text.split()
            if len(term.strip()) >= 3
            and term.strip() not in STOP_WORDS
            and term.strip() not in host_terms
        ]

        terms.extend(raw_terms)

        for expansion in INTENT_EXPANSIONS:
            if any(term in expansion["triggers"] for term in raw_terms):
                terms.extend(normalize_text(term) for term in expansion["terms"])

    return dedupe(terms)


def build_search_text(props: ElementProperties) -> str:
    attributes = props.get("attributes", {}) or {}
    return normalize_text(
        " ".join(
            value
            for value in [
                props.get("labelText"),
                props.get("ariaLabel"),
                props.get("textContent"),
                attributes.get("placeholder"),
                attributes.get("title"),
                attributes.get("name"),
                attributes.get("value"),
                attributes.get("id"),
                props.get("role"),
            ]
            if value
        )
    )


def selector_matches_props(selector: str, props: ElementProperties) -> bool:
    tag = (props.get("tagName") or "").lower()
    attributes = props.get("attributes", {}) or {}
    element_id = attributes.get("id")
    classes = [value for value in (attributes.get("class") or "").split() if value]

    parts = [part.strip() for part in selector.split(",")]
    for part in parts:
        if part.startswith("#") and element_id and part == f"#{element_id}":
            return True
        if part.startswith(".") and any(part == f".{name}" for name in classes):
            return True

        match = re.match(r"^(\w+)", part)
        if match and match.group(1).lower() == tag:
            return True

    return False


def matches_element(props: ElementProperties, rule: AclRule) -> bool:
    if not rule.get("selector") and not rule.get("textMatch") and not rule.get("description"):
        return False

    selector = rule.get("selector")
    if selector and not selector_matches_props(selector, props):
        return False

    compiled_terms = compile_acl_terms(rule)
    if not compiled_terms:
        return bool(selector)

    search_text = build_search_text(props)
    return any(term in search_text for term in compiled_terms)


def find_first_matching_rule(
    page_url: str,
    props: ElementProperties,
    rules: list[AclRule],
) -> AclRule | None:
    site_rules = [rule for rule in rules if matches_site_pattern(page_url, rule.get("sitePattern", ""))]
    if not site_rules:
        return None

    site_only = next(
        (
            rule
            for rule in site_rules
            if not rule.get("selector")
            and not rule.get("textMatch")
            and not rule.get("description")
        ),
        None,
    )
    if site_only:
        return site_only

    return next((rule for rule in site_rules if matches_element(props, rule)), None)
