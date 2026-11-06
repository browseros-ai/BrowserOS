"""What the evidence means: the judge half of the claw-sdk leak test.

leak_sdk.py records every byte that came back from a Neo container into an evidence
directory and decides nothing. This module reads that directory and decides everything:
which planted value appears where (detection), whether its absence proves anything
(presence and coverage), which answers are a contract refusing, which leaks are known
residuals, and the table the owner reads.

    evidence dir ─ Evidence.load ─▶ observe() ─▶ decide() ─▶ assign_residuals() ─▶ Judgement
                                   (bytes x plants)  (one rule)    (one table)      report.md,
                                                                                    verdicts.json,
                                                                                    exit status

It owns the evidence schema (the driver writes records only through `encode`), so a
re-judge reads exactly what the run read. Pure: files in, verdicts out; `--rejudge`
reruns it without Docker. The wire parsers it replays streams with live in leak_wire.
"""

from __future__ import annotations

import bisect
import collections
import contextlib
import dataclasses
import functools
import html
import json
import re
import urllib.parse
from collections.abc import Callable, Collection, Iterable, Iterator, Mapping, Sequence
from enum import StrEnum
from pathlib import Path

from leak_plan import (
    ANCHOR_LENGTH,
    COPY_BASE,
    COPY_DROP,
    DENY_CODE,
    ECHO_LIKE,
    FRAGMENT,
    HELD,
    KINDS,
    LONG_SPELLINGS,
    OPENER_PROBES,
    PLACES,
    SPECIAL_TAIL,
    SPECS,
    ChannelId,
    Exposure,
    Family,
    Instance,
    Kind,
    Layout,
    Location,
    Origin,
    Phase,
    Plant,
    Region,
    Secret,
)
from leak_wire import MjpegParser, RfbParser, SseParser, mp4_boxes, parse_run_reply

SCHEMA = 1

# ------------------------------------------------------------------------------------------
# Evidence: what the driver recorded. Append-only JSON lines plus blobs; never rewritten.
# ------------------------------------------------------------------------------------------


@dataclasses.dataclass(frozen=True)
class Request:
    """Exactly what the harness sent, enough to repeat it: the "exact leaking request".

    It never holds a planted value or an anchor (leak_plan.assert_clean ran on it),
    except the designed injections (owner keys, POST /v1/secrets), named by `inject`.
    """

    transport: str  # "mcp", "http", "ws"
    method: str  # "tools/call run", "GET", "POST", "WS"
    target: (
        str  # "/mcp", "/v1/operations/0195….3", "/v1/display?mode=view&followTab=12"
    )
    body: str | None = None  # the JSON-RPC request, or the HTTP body
    inject: str | None = None  # plant id when the request carries a value by design


@dataclasses.dataclass(frozen=True)
class Refusal:
    """A channel's "no", parsed at the boundary: a run error, a non-2xx, a close code."""

    owner: str  # "host", "script", "neo", "http", "ws", "harness"
    code: str  # "page_private", "method_unavailable", "409", "4001"
    message: str = ""

    @property
    def key(self) -> str:
        return f"{self.owner}/{self.code}"


@dataclasses.dataclass(frozen=True)
class Capture:
    """One request to one channel and every byte of its answer (in `blob`)."""

    id: str
    scenario: str
    instance: str | None
    channel: ChannelId
    phase: Phase
    request: Request
    blob: str  # path under the evidence dir
    started_ms: int
    ended_ms: int
    status: int | None = None  # HTTP status, for host routes
    run_id: str | None = None  # the agent run it answers or the host record it fetched
    proofs: tuple[str, str] | None = None  # oracle proofs just before and just after
    trigger: str | None = None  # the act fired while the run waited for its event
    replay_of: str | None = None  # the capture whose requestId this one repeated
    regions: tuple[str, ...] = ()  # what the run asked for; a region it did not ask
    # for is n/a unless the answer shows it anyway
    control: str | None = None  # a copy probe's positive control: its baseline copy run


@dataclasses.dataclass(frozen=True)
class Proof:
    """An oracle snapshot of one instance: an FNV-1a digest and length per region (None
    where the page does not show the region), and which documents answered."""

    id: str
    instance: str
    at_ms: int
    regions: Mapping[str, tuple[str, int] | None]
    documents: tuple[str, ...]
    # The ids of the focused elements, each from a document that has the focus (the
    # iframe on the way down, then the field): where keyboard input lands now.
    focus: tuple[str, ...] = ()


@dataclasses.dataclass(frozen=True)
class Step:
    """Something the harness made the page, the owner or Neo do, in order. Steps carry
    the facts the judge derives the rest from: a tab, a privacy start, a copy, a paste."""

    at_ms: int
    scenario: str
    instance: str | None
    action: str  # open, act, sweep, private, secret, takeover, release, control_open,
    # control_input, control_close, typed, copy, paste, close, failed, profile_scan
    detail: Mapping[str, object] = dataclasses.field(default_factory=dict)


@dataclasses.dataclass(frozen=True)
class Stream:
    """A long-lived answer: the events stream, a screencast, a video, an RFB display.
    Raw bytes in `blob`, with the host time each chunk arrived."""

    id: str
    scenario: str
    channel: ChannelId
    instance: str | None
    tab: int | None
    request: Request
    blob: str
    opened_ms: int
    chunks: tuple[tuple[int, int], ...]  # (byte offset, arrival ms)
    closed: tuple[int, str] | None  # when and why ("close 4001", "eof", "harness")
    status: int | None = None  # 200 / 101, or the refusal's HTTP status
    role: str = ""  # "view", "control", "retry" (a request made after privacy)
    geometry: tuple[int, int, str] | None = None  # RFB width, height, pixel format hex


@dataclasses.dataclass(frozen=True)
class Facts:
    """What preflight learned; the report states it and some verdicts depend on it."""

    image: str
    revision: str | None
    fx_f_in_revision: (
        bool | None
    )  # git: does the revision hold the site-wide read mask?
    oopif: Mapping[str, bool]  # document -> observed as its own target
    checks: Mapping[
        str, str
    ]  # preflight check -> observed answer ("ok ..." / "FAIL ...")
    clipboard_flags: str
    clock_offset_ms: int  # Neo's clock minus the host's
    tools: Mapping[str, str | None]  # tesseract, ffmpeg versions


Record = Capture | Proof | Step | Stream | Facts
_TYPES: Mapping[str, type] = {
    t.__name__: t for t in (Capture, Proof, Step, Stream, Facts)
}


def encode(record: Record) -> str:
    """One JSON line for log.jsonl, tagged with its type and SCHEMA."""
    return json.dumps(
        {"type": type(record).__name__, "schema": SCHEMA, **dataclasses.asdict(record)}
    )


def decode(line: str) -> Record:
    data = json.loads(line)
    cls = _TYPES[data.pop("type")]
    known = {f.name for f in dataclasses.fields(cls)}
    data = {k: v for k, v in data.items() if k in known}  # a re-judge outlives renames
    if cls is Capture:
        data["channel"] = ChannelId(data["channel"])
        data["phase"] = Phase(data["phase"])
        data["request"] = Request(**data["request"])
        data["proofs"] = tuple(data["proofs"]) if data["proofs"] else None
        data["regions"] = tuple(data.get("regions", ()))
    elif cls is Stream:
        data["channel"] = ChannelId(data["channel"])
        data["request"] = Request(**data["request"])
        data["chunks"] = tuple(tuple(c) for c in data["chunks"])
        data["closed"] = tuple(data["closed"]) if data["closed"] else None
        data["geometry"] = tuple(data["geometry"]) if data["geometry"] else None
    elif cls is Proof:
        data["regions"] = {
            k: tuple(v) if v else None for k, v in data["regions"].items()
        }
        data["documents"] = tuple(data["documents"])
        data["focus"] = tuple(data.get("focus", ()))
    return cls(**data)


def instance_to_json(instance: Instance) -> dict:
    """The manifest's record of an instance, values included: they are this run's
    synthetic values, and a re-judge needs them without the generator."""
    return {
        "name": instance.name,
        "layout": str(instance.layout),
        "plants": [
            {
                "id": p.id,
                "kind": str(p.kind),
                "location": str(p.location),
                "origin": str(p.origin),
                "value": p.secret.reveal(),
                "late": p.late,
                "navigates": p.navigates,
            }
            for p in instance.plants
        ],
        "regions": [
            {
                "id": r.id,
                "anchor": r.anchor,
                "plant": r.plant.id,
                "exposure": str(r.exposure),
                "location": str(r.location),
                "part": r.part,
            }
            for r in instance.regions
        ],
    }


def instance_from_json(data: Mapping) -> Instance:
    plants = {
        p["id"]: Plant(
            p["id"],
            Kind(p["kind"]),
            Location(p["location"]),
            Origin(p["origin"]),
            Secret(p["value"]),
            late=p["late"],
            navigates=p["navigates"],
        )
        for p in data["plants"]
    }
    regions = tuple(
        Region(
            r["id"],
            r["anchor"],
            plants[r["plant"]],
            Exposure(r["exposure"]),
            Location(r["location"]),
            r["part"],
        )
        for r in data["regions"]
    )
    return Instance(
        data["name"], Layout(data["layout"]), tuple(plants.values()), regions
    )


@dataclasses.dataclass(frozen=True)
class Evidence:
    """Everything one run recorded. Loading never contacts the container."""

    root: Path
    seed: str
    image: str
    instances: Mapping[str, Instance]
    captures: tuple[Capture, ...]
    proofs: Mapping[str, Proof]
    steps: tuple[Step, ...]
    streams: tuple[Stream, ...]
    facts: Facts | None  # None when preflight never finished
    # capture or stream id -> its 1-based line in log.jsonl: where the report sends a
    # reader for the exact request, which it shows cut
    lines: Mapping[str, int] = dataclasses.field(default_factory=dict)

    @classmethod
    def load(cls, root: Path) -> Evidence:
        """manifest.json and log.jsonl; a partial directory from a crashed run loads too
        (a torn last line is dropped)."""
        manifest = json.loads((root / "manifest.json").read_text())
        records: list[Record] = []
        lines: dict[str, int] = {}
        log = root / "log.jsonl"
        for n, line in enumerate(
            log.read_text().splitlines() if log.exists() else (), 1
        ):
            try:
                record = decode(line)
            except (ValueError, KeyError, TypeError):
                continue
            records.append(record)
            if isinstance(record, Capture | Stream):
                lines[record.id] = n
        facts = [r for r in records if isinstance(r, Facts)]
        return cls(
            root=root,
            seed=manifest["seed"],
            image=manifest["image"],
            instances={i["name"]: instance_from_json(i) for i in manifest["instances"]},
            captures=tuple(r for r in records if isinstance(r, Capture)),
            proofs={r.id: r for r in records if isinstance(r, Proof)},
            steps=tuple(r for r in records if isinstance(r, Step)),
            streams=tuple(r for r in records if isinstance(r, Stream)),
            facts=facts[-1] if facts else None,
            lines=lines,
        )

    def blob(self, path: str) -> bytes:
        return (self.root / path).read_bytes()

    @property
    def failures(self) -> tuple[str, ...]:
        return tuple(
            f"{s.scenario}: {s.detail.get('error', '')}"
            for s in self.steps
            if s.action == "failed"
        )


# ------------------------------------------------------------------------------------------
# Detection
# ------------------------------------------------------------------------------------------


def fnv1a(text: str) -> str:
    """FNV-1a 32 over UTF-8, as hex: the digest fx.js reports, so presence compares
    without the oracle ever carrying a value."""
    h = 0x811C9DC5
    for b in text.encode():
        h = ((h ^ b) * 0x01000193) & 0xFFFFFFFF
    return f"{h:08x}"


def live(proof: Proof | None, region: Region) -> bool:
    """The proof shows the region holding exactly its plant's value."""
    if proof is None:
        return False
    expected = region.expected()
    return proof.regions.get(region.id) == (fnv1a(expected), len(expected))


class Form(StrEnum):
    """How a found value was spelled."""

    PLAIN = "plain"  # also every escaped form of an alphanumeric value
    JSON = "json"  # the SPECIAL tail JSON-escaped
    HTML = "html"  # the SPECIAL tail HTML-escaped
    URL = "url"  # the SPECIAL tail percent-encoded
    GROUPED = "grouped"  # a card number in 4-digit groups
    FRAGMENT = "fragment"  # >= 8 consecutive characters of a value, not all of it
    SLOT = "slot"  # an anchored slot holding the raw value (how short values are found)


@dataclasses.dataclass(frozen=True)
class Hit:
    plant: str
    region: str | None  # attributed region: by key, by the nearest anchor, or None
    form: Form
    offset: int
    context: str  # +-60 characters, every planted value replaced by its placeholder


class SlotState(StrEnum):
    RAW = "raw"  # the region's value: a leak, whatever its length
    MASKED = "masked"  # ••••, •••• dddd, [private], [redacted]
    EMPTY = "empty"
    OTHER = "other"  # neither: the probe read something else; inconclusive


_ALNUM = re.compile(r"[A-Za-z0-9]{8,}")
_DIGITS = re.compile(r"(?<![0-9])[0-9]{13,19}(?![0-9])")
_GROUPED = re.compile(r"(?<![0-9])[0-9]{4}(?:[ -][0-9]{4}){3}(?![0-9])")
_REGION_KEY = re.compile(r"r\d{5}")
_KEYED_RAW = re.compile(r'"(r\d{5})":\{"ok":"((?:[^"\\]|\\.)*)"')
# Mask tokens: L1's bullets, L2's [private], the trace's [redacted]. A slot made of them
# alone is masked, also in combination (L2 can mask the last four L1 shows of a card).
_MASK_TOKENS = re.compile(r"•+|\[private\]|\[redacted\]")
_ATTRIBUTION = 160  # how far from a value its region's key or anchor may sit
# Where a short value sits in a slot the judge reads, as raw text in whatever escaping
# wraps it (any run of backslashes before a quote, one per level of JSON nesting): an
# echo's braces, also URL-encoded; a keyed read's, a getByRef entry's or an aria node's
# JSON string ("ok", "text"); an aria line's value and its child "- text:" line; an
# <input>'s value attribute. The redactor's reading of every form slots() reads.
_SLOT_VALUE = re.compile(
    r"\{\{([^\n{}]{0,64}?)\}\}"
    r"|%7[Bb]%7[Bb](.{0,192}?)%7[Dd]%7[Dd]"
    r'|\\*"(?:ok|text)\\*": ?\\*"([^"\\]{0,64})'
    r'|(?:\]|\\*"):[ ]([^"\\\n]{0,64})'
    r'|- text: ([^"\\\n]{0,64})'
    r'|\bvalue=\\*"([^"\\]{0,64})'
    r'|Received(?: string)?: +\\*"([^"\\]{0,64})'
    r'|unexpected value \\*"([^"\\\n]{0,64})'
    r'|\.type\(\\*"([^"\\]{0,64})'
)
# An expect failure's Received line, and the `unexpected value` line each poll adds to
# its call log (browseros-playwright expect/matcher_message.rs): Received escapes only
# `"` and `\`, a poll line prints the value bare. In a keyed error they are the region's.
_RECEIVED = re.compile(r'^Received(?: string)?: +"((?:[^"\\\n]|\\.)*)"$', re.M)
_UNEXPECTED = re.compile(r'unexpected value "(.*)"$', re.M)
# What a typing run carried, as its records keep it: the typed argument of a typing op
# (the trace redacts it for a sensitive target) and the literal in the run's code (which
# the trace withholds once an op was sensitive). leak_plan.typing_code writes the literal.
_TYPING_ARGS = {
    "keyboard.type": "text",
    "keyboard.insertText": "text",
    "locator.pressSequentially": "text",
    "locator.fill": "value",
}
_TYPED_LITERAL = re.compile(r'keyboard\.type\(("(?:[^"\\]|\\.)*")\)')


@dataclasses.dataclass(frozen=True)
class NeoSecret:
    """One of Neo's own secrets that the harness mints (leak_sdk.Session) and no plant
    holds, known by its shape. Every answer the judge reads is hunted for it
    (_Observer.take): a hit is a new leak whose cell is about `subject`. The redactor
    prints `shown` in its place, so the report never holds it."""

    pattern: re.Pattern
    shown: str
    subject: Region


def _neo_secret(name: str, pattern: str, shown: str) -> NeoSecret:
    # A NEO plant of no page. The detector never indexes it, so its placeholder value
    # is never searched; its cells take phase ANY, which no phase-bound residual fits.
    plant = Plant(f"neo.{name}", Kind.FILE, Location.MAIN, Origin.NEO, Secret(shown))
    region = Region(f"neo-{name}", f"neo-{name}", plant, Exposure.FIELD, Location.MAIN)
    return NeoSecret(re.compile(pattern), shown, region)


# No byte of Neo's other secret files reaches the evidence (leak_plan.deny_uploads).
NEO_SECRETS = (
    _neo_secret("host-token", r"leak-[0-9a-f]{48}", "⟨host token⟩"),
    _neo_secret("captcha-key", r"leak-key-[0-9a-f]{40}", "⟨CAPTCHA key⟩"),
)
_SPECIAL_FORMS = {
    Form.JSON: json.dumps(SPECIAL_TAIL)[1:-1],
    Form.HTML: html.escape(SPECIAL_TAIL),
    Form.URL: urllib.parse.quote(SPECIAL_TAIL),
}


def placeholder(plant: Plant, region: Region | None = None) -> str:
    """What the report prints instead of a value: ⟨i07.p03 cc_csc@XO⟩, where XO is
    where the plant's field is (the region's own place is printed beside it)."""
    return f"⟨{plant.id} {plant.kind}@{PLACES[plant.location].code}⟩"


def _spellings(value: str) -> set[str]:
    return {
        value,
        json.dumps(value)[1:-1],
        json.dumps(value),
        html.escape(value),
        html.escape(value, quote=False),
        urllib.parse.quote(value),
        urllib.parse.quote_plus(value),
    }


def classify(slot: str, region: Region) -> SlotState:
    expected = region.expected()
    text = slot.strip()
    spellings = _spellings(expected)
    if text in spellings:
        return SlotState.RAW
    if not text:
        return SlotState.EMPTY
    rest = _MASK_TOKENS.sub("", text).strip()
    if text != rest and (
        not rest or (rest.isdigit() and len(rest) == 4 and expected.endswith(rest))
    ):
        return SlotState.MASKED  # mask tokens, and at most a card's last four
    if len(expected) >= 4 and any(s in text for s in spellings):
        return SlotState.RAW
    if _JSON_STRING.fullmatch(text):
        # A main-world script's JSON.stringify(value), which D3's guard masks inside the
        # quotes: what the string spells decides, by the same rules.
        with contextlib.suppress(ValueError):
            return classify(json.loads(text), region)
    return SlotState.OTHER


_JSON_STRING = re.compile(r'"(?:[^"\\\x00-\x1f]|\\.)*"')
_CUT = re.compile(r"…\[\+\d+ bytes\]$")  # how the trace store marks a truncated string
_MASK_HEADS = ("[private]", "[redacted]", "•••• ")


def classify_cut(slot: str, region: Region) -> SlotState | None:
    """classify(), for a slot the trace store may have truncated: a bare truncation
    marker holds nothing of the value (no slot); a head before the marker is raw when it
    starts the value and masked when it starts a mask."""
    cut = _CUT.search(slot)
    if cut is None:
        return classify(slot, region)
    head = slot[: cut.start()]
    if not head:
        return None
    if region.expected().startswith(head):
        return SlotState.RAW
    masks = any(m.startswith(head) or head.startswith(m) for m in _MASK_HEADS)
    return SlotState.MASKED if set(head) <= {"•"} or masks else SlotState.OTHER


@functools.cache
def _standalone(value: str) -> re.Pattern:
    """A short value as a whole run of digits: "12" inside "2012" is not it."""
    return re.compile(rf"(?<!\d){re.escape(value)}(?!\d)")


@functools.cache
def _patterns(anchor: str, region_id: str) -> tuple[re.Pattern, re.Pattern, re.Pattern]:
    """A region's slot patterns, compiled once: its echo slot, its aria textbox line,
    and its <input> tag.

    An echo slot holds at most a value's spelling: a hint cut before its closing braces,
    or a string split across sqlite pages, is no slot at all. A textbox line carries its
    value after ": ", or, when the field shows a placeholder, on a child "- text:" line
    below the placeholder's.
    """
    a = re.escape(anchor)
    echo = re.compile(rf"ECHO {a} \{{\{{([^\x00-\x1f]{{0,64}}?)\}}\}}")
    aria = re.compile(
        rf'"[^"\n]*\b{a}\b[^"\n]*"(?: \[[^\]\n]*\])*:(?: ([^\x00-\x1f"]*)|\n'
        r'(?:[ ]+- /[^\n]*\n)*[ ]+- text: ([^\x00-\x1f"]*))?'
    )
    tag = re.compile(rf'<input\b[^>]*\bid="{re.escape(region_id)}"[^>]*>')
    return echo, aria, tag


@dataclasses.dataclass(frozen=True)
class Views:
    """One answer, readable three ways: its raw text (long values in every spelling),
    its decoded strings (aria lines, echo slots, markup), and its keyed reads.

    A region's keyed read can occur more than once (a run reply carries its value in
    structuredContent and again in its text block; a disk page can hold old and new
    copies), and the copies can disagree. Every copy is kept, so a raw one is never
    hidden behind a masked one.
    """

    raw: str
    strings: tuple[str, ...]
    # region id -> each copy of its read: {ok} | {err, owner, code}
    keyed: Mapping[str, Sequence[Mapping]]
    refs: tuple[Mapping, ...]  # getByRef entries: {line, ok | err}
    nodes: tuple[Mapping, ...] = ()  # aria nodes as host records store them
    # An element script's own answer, {near, <spelling>...}, as an op's record keeps it:
    # the run regroups it by region id for its reply, but the op's result names the
    # element only by its label's anchor in `near`.
    answers: tuple[Mapping, ...] = ()
    # region id -> its slot state in this answer: the slot hit scan and the per-region
    # verdicts both ask, and a disk page's slots cost a pass over megabytes each.
    states: dict[str, SlotState | None] = dataclasses.field(
        default_factory=dict, compare=False, repr=False
    )


def matcher_slots(message: str) -> list[str]:
    """The values an expect failure's message shows: its Received value and each poll's."""
    received = [re.sub(r"\\(.)", r"\1", m) for m in _RECEIVED.findall(message)]
    return received + _UNEXPECTED.findall(message)


def typed_slots(text: str) -> list[str]:
    """Every place a record of a typing run holds what the run typed."""
    out: list[str] = []
    for tree in _json_trees(text):
        for node in _leaves(tree):
            if not isinstance(node, dict):
                continue
            name, args = node.get("name"), node.get("args")
            arg = _TYPING_ARGS.get(name) if isinstance(name, str) else None
            if arg and isinstance(args, dict) and isinstance(args.get(arg), str):
                out.append(args[arg])
            if isinstance(node.get("code"), str):
                for m in _TYPED_LITERAL.finditer(node["code"]):
                    with contextlib.suppress(ValueError):
                        out.append(json.loads(m[1]))
    return out


def _worst(states: Iterable[SlotState | None]) -> SlotState | None:
    """The slot state a region shows when it has several slots: a raw one wins."""
    found = set(states)
    ranked = (SlotState.RAW, SlotState.OTHER, SlotState.MASKED, SlotState.EMPTY)
    return next((s for s in ranked if s in found), None)


def _leaves(tree: object) -> Iterator[object]:
    if isinstance(tree, dict):
        yield tree
        for value in tree.values():
            yield from _leaves(value)
    elif isinstance(tree, list):
        for value in tree:
            yield from _leaves(value)
    else:
        yield tree


def _json_trees(text: str) -> list[object]:
    """The answer as JSON: one body, or one per SSE data payload (MCP and /v1/events)."""
    try:
        return [json.loads(text)]
    except ValueError:
        pass
    trees = []
    for line in text.splitlines():
        if line.startswith("data:"):
            try:
                trees.append(json.loads(line[5:]))
            except ValueError:
                continue
    return trees


def _plain(node: object) -> object:
    """`node` with every serialized value decoded to plain JSON.

    A main-world script's result reaches the host's records (/v1/runs, /v1/operations,
    /v1/trace, /v1/events, trace.db) in the form the crate serializes it: an object as
    {id, o: [{k, v}]} and an array as {id, a: [...]}, each cut one ending in {"$more": n}.
    Its region keys then sit in "k" values, not in keys, and only decoded does a record
    hold the keyed read the reply holds, so a short value is judged in the record by the
    same slot."""
    if isinstance(node, list):
        return [_plain(item) for item in node]
    if not isinstance(node, dict):
        return node
    shape = node.keys() - {"id"}
    if shape == {"o"} and isinstance(node["o"], list):
        return {
            entry["k"]: _plain(entry.get("v"))
            for entry in node["o"]
            if isinstance(entry, dict) and isinstance(entry.get("k"), str)
        }
    if shape == {"a"} and isinstance(node["a"], list):
        return [
            _plain(item)
            for item in node["a"]
            if not (isinstance(item, dict) and item.keys() == {"$more"})
        ]
    return {key: _plain(value) for key, value in node.items()}


def views_of(data: bytes, *, structured: bool = True) -> Views:
    """The answer's views. A JSON string that itself holds JSON (a run's text block is
    its value serialized once more) is replaced by what it holds, so slot patterns never
    run over escaped text. Bytes that are not JSON (disk pages, logs) are read with JSON
    escapes undone."""
    text = data.decode("utf-8", "replace")
    trees = [_plain(tree) for tree in _json_trees(text)] if structured else []
    keyed: dict[str, list[Mapping]] = {}
    if not trees:
        unescaped = text.replace('\\"', '"').replace("\\n", "\n")
        for m in _KEYED_RAW.finditer(text):
            with contextlib.suppress(ValueError):  # a keyed read cut by a page boundary
                keyed.setdefault(m[1], []).append({"ok": json.loads(f'"{m[2]}"')})
        return Views(text, (unescaped,), keyed, ())
    strings: list[str] = []
    refs: list[Mapping] = []
    nodes: list[Mapping] = []
    answers: list[Mapping] = []
    pending = [n for tree in trees for n in _leaves(tree)]
    while pending:
        node = pending.pop()
        if isinstance(node, str):
            inner = _json_trees(node) if node[:1] in '{["' else []
            if inner:
                pending += [n for tree in inner for n in _leaves(_plain(tree))]
            else:
                strings.append(node)
        elif isinstance(node, dict):
            for key, value in node.items():
                if _REGION_KEY.fullmatch(key) and isinstance(value, dict):
                    keyed.setdefault(key, []).append(value)
            if "line" in node and ("ok" in node or "err" in node):
                refs.append(node)
            if node.get("role") == "textbox" and isinstance(node.get("name"), str):
                nodes.append(node)
            if isinstance(node.get("near"), str) and node.keys() & set(LONG_SPELLINGS):
                answers.append(node)
    return Views(text, tuple(strings), keyed, tuple(refs), tuple(nodes), tuple(answers))


class Detector:
    """Finds every plant of the run in any answer.

    Long values: one pass over the text's alphanumeric runs ([A-Za-z0-9]{8,}), each 8-gram
    looked up in an index of every TEXT value's core -> the full value or a fragment of 8+
    characters. Escaping leaves alphanumerics alone, so one pass finds every spelling.
    Card numbers: digit runs of 13-19 compared whole, and the grouped forms; never
    fragments, because 8-digit windows match timestamps. Short values (CVC, split digits,
    a joined split code): only in anchored slots, so "123" in a timestamp never counts.
    Anchors and region ids are unique across the run, so a slot names its region, and a
    raw one is a hit whichever instance it belongs to (slot_hits).
    """

    def __init__(self, instances: Iterable[Instance]) -> None:
        self.plants: dict[str, Plant] = {}
        self.regions: dict[str, Region] = {}
        self.anchors: dict[str, Region] = {}
        self.cores: dict[str, str] = {}
        self.grams: dict[str, tuple[str, int]] = {}
        self.cards: dict[str, str] = {}
        for instance in instances:
            for region in instance.regions:
                self.regions[region.id] = region
                self.anchors[region.anchor] = region
            for plant in instance.plants:
                self.plants[plant.id] = plant
                value = plant.secret.reveal()
                if KINDS[plant.kind].shape.value == "card" and value.isdigit():
                    self.cards[value] = plant.id
                elif len(value) >= FRAGMENT and value[0].isalpha():
                    core = value.removesuffix(SPECIAL_TAIL)
                    self.cores[plant.id] = core
                    for i in range(len(core) - FRAGMENT + 1):
                        gram = core[i : i + FRAGMENT]
                        if gram in self.grams and self.grams[gram][0] != plant.id:
                            raise ValueError(f"{plant.id} shares an 8-gram: reseed")
                        self.grams[gram] = (plant.id, i)
        self._anchor_rx = re.compile("|".join(map(re.escape, self.anchors)) or "(?!)")
        # Every window of anchor characters, overlapping, so one C-speed pass lists the
        # anchors a text holds (also one glued to a neighbour, as in "%20<anchor>").
        alphabet = re.escape("".join(sorted({c for a in self.anchors for c in a})))
        self._anchor_scan = re.compile(
            f"(?=([{alphabet}]{{{ANCHOR_LENGTH}}}))" if alphabet else "(?!)"
        )

    def redact(self, text: str) -> str:
        """`text` with every planted value replaced by its placeholder: every long value
        in any spelling (or a fragment of 8+), every card, and every short value in an
        anchored slot. The one redactor: everything report.md and verdicts.json print
        passes through it."""
        out, at = [], 0
        for start, end, shown in self._spans(text):
            out += [text[at:start], shown]
            at = end
        return "".join(out) + text[at:]

    def context(self, text: str, start: int, end: int) -> str:
        """What the report shows of `text` around [start, end): 60 characters each side,
        redacted. The redactor reads a window wider by a slot's reach first, so a slot
        whose anchor the cut leaves out is still recognised, and a value the cut would
        split is shown whole as its placeholder."""
        lo, hi = max(0, start - 60), min(len(text), end + 60)
        base = max(0, lo - _ATTRIBUTION)
        window = text[base : hi + _ATTRIBUTION]
        spans = self._spans(window)
        a, b = lo - base, hi - base
        for s, e, _ in spans:
            a, b = (s if s < a < e else a), (e if s < b < e else b)
        out, at = [], a
        for s, e, shown in spans:
            if a <= s and e <= b:
                out += [window[at:s], shown]
                at = e
        return "".join(out) + window[at:b]

    def _spans(self, text: str) -> list[tuple[int, int, str]]:
        """Where `text` holds a planted value, as (start, end, placeholder): sorted and
        disjoint. A short value counts only inside a slot (_SLOT_VALUE) with its region's
        anchor or id within _ATTRIBUTION either side: escaping and key order can put
        either one first (an <input>'s value before its id)."""
        spans = [
            (start, end, placeholder(self.plants[pid]))
            for start, end, pid in self._raw_hits(text)
        ]
        spans += [
            (*m.span(), s.shown) for s in NEO_SECRETS for m in s.pattern.finditer(text)
        ]
        marks = [(at, region) for at, region in self._marks(text) if region.short]
        where = [at for at, _ in marks]
        for m in _SLOT_VALUE.finditer(text) if marks else ():
            start, end = m.span(m.lastindex)
            lo = bisect.bisect_left(where, start - _ATTRIBUTION)
            hi = bisect.bisect_right(where, end + _ATTRIBUTION)
            for _, region in marks[lo:hi]:
                for v in _standalone(region.expected()).finditer(text, start, end):
                    spans.append((v.start(), v.end(), placeholder(region.plant)))
        merged: list[tuple[int, int, str]] = []
        for span in sorted(spans, key=lambda span: (span[0], -span[1])):
            if merged and span[0] < merged[-1][1]:
                if span[1] > merged[-1][1]:  # overlapping values: one placeholder
                    merged[-1] = (merged[-1][0], span[1], merged[-1][2])
                continue
            merged.append(span)
        return merged

    def _raw_hits(self, text: str) -> list[tuple[int, int, str]]:
        found: list[tuple[int, int, str]] = []
        for m in _ALNUM.finditer(text):
            run, base, i = m.group(), m.start(), 0
            while i + FRAGMENT <= len(run):
                entry = self.grams.get(run[i : i + FRAGMENT])
                if entry is None:
                    i += 1
                    continue
                pid, offset = entry
                core = self.cores[pid]
                left, k = i, offset
                while left > 0 and k > 0 and run[left - 1] == core[k - 1]:
                    left, k = left - 1, k - 1
                right, k = i + FRAGMENT, offset + FRAGMENT
                while right < len(run) and k < len(core) and run[right] == core[k]:
                    right, k = right + 1, k + 1
                found.append((base + left, base + right, pid))
                i = right
        for rx in (_DIGITS, _GROUPED):
            for m in rx.finditer(text):
                pid = self.cards.get(re.sub(r"[ -]", "", m.group()))
                if pid:
                    found.append((m.start(), m.end(), pid))
        return found

    def hits(self, text: str) -> list[Hit]:
        out = []
        for start, end, pid in self._raw_hits(text):
            plant = self.plants[pid]
            if plant.id in self.cores:
                full = end - start == len(self.cores[pid])
                special = plant.secret.reveal().endswith(SPECIAL_TAIL)
                tail = text[end : end + 24]
                form = Form.FRAGMENT
                if full and (not special or tail.startswith(SPECIAL_TAIL)):
                    form = Form.PLAIN
                for spelled, spelling in _SPECIAL_FORMS.items():
                    if full and special and tail.startswith(spelling):
                        form = spelled
            else:
                form = Form.PLAIN if text[start:end].isdigit() else Form.GROUPED
            out.append(
                Hit(
                    pid,
                    self._attribute(text, start, plant),
                    form,
                    start,
                    self.context(text, start, end),
                )
            )
        return out

    def _marks(self, text: str) -> list[tuple[int, Region]]:
        """Where the text names a region, by its anchor or its id, in text order."""
        anchors = (m for m in self._anchor_scan.finditer(text) if m[1] in self.anchors)
        ids = (m for m in _REGION_KEY.finditer(text) if m[0] in self.regions)
        marks = [(m.start(), self.anchors[m[1]]) for m in anchors]
        marks += [(m.start(), self.regions[m[0]]) for m in ids]
        return sorted(marks, key=lambda mark: mark[0])

    def named(self, text: str) -> list[Region]:
        """Every region whose anchor or id the text holds, in run order."""
        regions = {region.id: region for _, region in self._marks(text)}
        return [regions[r] for r in sorted(regions)]

    def slot_hits(self, views: Views) -> list[Hit]:
        """Every short region, of any instance, whose anchored slot in this answer holds
        its raw value. A short value has no free-text needle, so this is how it is
        found, whoever asked and whether or not the oracle showed the region."""
        out = []
        for region in self.named(views.raw):
            if region.short and self.slot_state(views, region) is SlotState.RAW:
                at = self._slot_offset(views.raw, region)
                context = self.context(views.raw, at, at + ANCHOR_LENGTH)
                out.append(Hit(region.plant.id, region.id, Form.SLOT, at, context))
        return out

    def _slot_offset(self, raw: str, region: Region) -> int:
        """Where the report's context for a raw slot starts: the anchor or id that has
        the raw value soonest after it, else the first one (it is only for reading)."""
        value = re.compile(rf"(?<!\d){re.escape(region.expected())}(?!\d)")
        tokens = (re.escape(region.anchor), re.escape(region.id))
        marks = [m.start() for t in tokens for m in re.finditer(t, raw)]
        for at in sorted(marks):
            if value.search(raw, at, at + _ATTRIBUTION):
                return at
        return min(marks, default=0)

    def _attribute(self, text: str, offset: int, plant: Plant) -> str | None:
        """The region a free-text hit belongs to: the nearest region key or anchor of the
        same plant within _ATTRIBUTION characters before it, else None (unanchored)."""
        window = text[max(0, offset - _ATTRIBUTION) : offset]
        found = None
        for m in re.finditer(r'"(r\d{5})"', window):
            region = self.regions.get(m[1])
            if region and region.plant is plant:
                found = region.id
        for m in self._anchor_rx.finditer(window):
            region = self.anchors[m.group()]
            if region.plant is plant:
                found = region.id
        return found

    def slots(self, views: Views, region: Region) -> list[str]:
        """Every slot of `region` the answer holds. An echo's slot is what sits between
        its anchor's {{ }}, wherever the text shows up; a field's is a plain keyed read
        (inputValue, getAttribute, getByRef), its aria line, its value attribute, or
        each spelling of an element script's answer whose label holds its anchor.
        Each pattern needs the region's anchor or id in the text, so a text without them
        is skipped before any pattern runs."""
        if region.anchor not in views.raw and region.id not in views.raw:
            return []
        out: list[str] = []
        echo, aria, tag = _patterns(region.anchor, region.id)
        if region.exposure in ECHO_LIKE:
            for s in views.strings:
                if region.anchor not in s:
                    continue
                for text in {s, urllib.parse.unquote(s), urllib.parse.unquote_plus(s)}:
                    out += echo.findall(text)
            return out
        reads = list(views.keyed.get(region.id, ()))
        reads += [r for r in views.refs if region.anchor in str(r.get("line", ""))]
        for answer in views.answers:
            if region.anchor in answer["near"]:
                out += [
                    answer[s] for s in LONG_SPELLINGS if isinstance(answer.get(s), str)
                ]
        for read in reads:
            if isinstance(read.get("ok"), str) and "<" not in read["ok"]:
                out.append(read["ok"])  # markup (innerHTML) is read below, by attribute
            if isinstance(read.get("err"), str):
                out += matcher_slots(read["err"])  # an expect failure about this region
        for node in views.nodes:
            if region.anchor in node["name"]:
                children = [
                    c
                    for c in node.get("children") or []
                    if isinstance(c, dict) and c.get("role") == "text"
                ]
                texts = [node.get("text")] + [c.get("text") for c in children]
                out += [t for t in texts if isinstance(t, str)][:1]
        for s in views.strings:
            if region.anchor in s:
                out += [inline or child for inline, child in aria.findall(s)]
            if region.id in s:
                for m in tag.finditer(s):
                    # Page bytes inside the attribute: a disk page cut it, no slot.
                    value = re.search(r'\bvalue="([^"\x00-\x1f]*)"', m.group())
                    if value:
                        out.append(html.unescape(value[1]))
        return out

    def slot_state(self, views: Views, region: Region) -> SlotState | None:
        if region.id not in views.states:
            views.states[region.id] = _worst(
                classify_cut(slot, region) for slot in self.slots(views, region)
            )
        return views.states[region.id]

    def covered(self, views: Views, region: Region) -> bool:
        """The answer proves the channel read this region: its anchor is there, a
        successful keyed read of its id, or a slot of its own: its element's preview with
        a value attribute, or the Received line of an expect failure keyed by its id."""
        reads = views.keyed.get(region.id, ())
        return (
            region.anchor in views.raw
            or any("ok" in read for read in reads)
            or self.slot_state(views, region) is not None
        )


# ------------------------------------------------------------------------------------------
# Verdicts
# ------------------------------------------------------------------------------------------


class Verdict(StrEnum):
    MASKED = "masked"  # proven there, proven read, and absent from the answer
    WITHHELD = "withheld"  # the channel refused by contract (held, private, paused)
    LEAKED = (
        "leaked"  # the value (any spelling, a fragment, a raw slot) is in the bytes
    )
    NOT_APPLICABLE = (
        "n/a"  # the channel cannot carry this region by contract, and did not
    )
    INCONCLUSIVE = (
        "inconclusive"  # no presence proof, no coverage, or an unexpected answer
    )


@dataclasses.dataclass(frozen=True)
class Pixels:
    """What a pixel channel showed about one private era."""

    private_since: int | None  # host ms the tab went private (None: never private)
    before: int  # frames delivered before it: the positive control
    after: int  # frames delivered at or after it (beyond the grace)
    first_after_ms: int | None = None
    unplaced: int = 0  # frames no record places before or after it


class Pasted(StrEnum):
    """What the paste box held after a copy probe's paste, by the oracle's digest."""

    BASELINE = (
        "baseline"  # the token the positive control copied: the chord copied nothing
    )
    VALUE = "value"  # the field's value: laundered through the clipboard
    EMPTY = "empty"  # nothing pasted: the round trip failed
    OTHER = "other"  # something else, such as an earlier probe's token
    UNREAD = "unread"  # no proof showed the paste box


@dataclasses.dataclass(frozen=True)
class Clip:
    """What a copy probe proved about the clipboard (decide_copy). The page put a fresh
    token in a plain field; the agent copied it (the positive control), sent the chord in
    the field, then pasted into an empty box, whose digest the oracle took after."""

    baseline_sent: bool  # the positive control's Control+C went out, not refused
    fresh: bool  # its token was new for this probe, so a paste of it is this probe's
    pasted: Pasted
    # The field had the keyboard focus just before the chord. Without it the chord could
    # land in the baseline field and copy its token again: "nothing copied" by accident.
    focused: bool = True


@dataclasses.dataclass(frozen=True)
class NeoRead:
    """What a deny-list probe proved beside its answer (decide_neo_upload), from the step
    the harness recorded: whether it could run, whether the refusal named a path of it,
    and what the oracle found the page read, compared by sha256 inside the container."""

    probe: str  # "host token by a symlink in /tmp"
    refused: bool = True  # the deny list must refuse it (a symlink directory is sent)
    front: bool = False  # the agent brought its tab forward before the upload
    present: bool = True  # its target is a file in this container
    made: bool = True  # the links that name it were made
    why: str = ""  # why not, when they were not
    names_path: bool = False  # the refusal's message names the path or its target's
    checked: bool = False  # the oracle looked at the page's input after the run
    files: int = 0  # files the page's input held
    read: int = 0  # of them, the ones the page read whole
    errors: tuple[str, ...] = ()  # why the page could not read the others
    matched: bool = False  # one it read hashes the same as a file the probe names
    empty: bool = False  # every file the probe names is empty: nothing to read back
    cancelled: bool = False  # the input fired `cancel`: Chromium set no file
    decoy: bool = False  # it read the decoy beside the symlink: the directory landed


@dataclasses.dataclass(frozen=True)
class Observation:
    """One region as one channel answered it: the input to the verdict rule."""

    channel: ChannelId
    region: Region
    phase: Phase
    source: str  # capture, stream or file id
    hit: Hit | None = None
    slot: SlotState | None = None
    covered: bool = False
    reachable: bool = False
    present: bool | None = True  # None: not shown in both proofs
    refusal: Refusal | None = None
    pixels: Pixels | None = None
    clip: Clip | None = None  # a copy probe's: judged by decide_copy
    neo: NeoRead | None = None  # a deny-list probe's: judged by decide_neo_upload
    # The page showed what the op handed it (the harness's "uploaded" step, by plant).
    received: bool = False
    run_id: str | None = None
    derived_from: str | None = None  # a record copying this run's agent answer
    withheld: bool = False  # the run's whole value was WITHHELD_ANSWER


# What claw-sdk's RunMask (dispatch/run_mask.rs) answers in place of a run's whole value:
# when one text matches both the process's secret set and the values the run holds, or
# when the run may have read a value the set lost. The answer then shows no region at all,
# so a cell it answers is held by contract, not unproven for want of its region.
WITHHELD_ANSWER = "[withheld]"


def decide(o: Observation) -> tuple[Verdict, str]:
    """The one verdict rule, with a reason for the report.

    found (hit, or slot RAW)                -> LEAKED   values are fresh and never sent
                                                         by the harness: found = leaked
    an op that answers nothing by contract  -> WITHHELD, N/A (not built) or INCONCLUSIVE
                                                         as it was refused; else N/A (ran),
                                                         or INCONCLUSIVE when what it had to
                                                         hand the page never showed there
    unreachable by contract and not covered -> N/A (not built, if refused so)
    refusal that the contract names         -> WITHHELD
    refusal that means "not built"          -> N/A (not built)
    any other refusal                       -> INCONCLUSIVE
    the run's whole value withheld          -> WITHHELD
    no presence proof bracketing the probe  -> INCONCLUSIVE
    not covered                             -> INCONCLUSIVE
    slot OTHER, or no slot for a short value-> INCONCLUSIVE
    otherwise                               -> MASKED
    """
    spec = SPECS[o.channel]
    if o.hit and o.hit.form is not Form.SLOT:
        return Verdict.LEAKED, f"{o.hit.form} value in the answer"
    if o.hit or o.slot is SlotState.RAW:
        return Verdict.LEAKED, "raw in its slot"
    refused = o.refusal is not None and o.refusal.key not in spec.answer_errors
    if spec.void and not refused:
        if spec.not_received and o.reachable and not o.received:
            return Verdict.INCONCLUSIVE, spec.not_received
        return Verdict.NOT_APPLICABLE, "ran; the op answers nothing by contract"
    if not o.reachable and not o.covered:  # before the refusal: it is not about them
        if refused and o.refusal.key in spec.not_built:
            return Verdict.NOT_APPLICABLE, f"not built ({o.refusal.key})"
        return Verdict.NOT_APPLICABLE, "unreachable by contract"
    if refused:
        if o.refusal.key in spec.refusals:
            return Verdict.WITHHELD, o.refusal.key
        if o.refusal.key in spec.not_built:
            return Verdict.NOT_APPLICABLE, f"not built ({o.refusal.key})"
        return (
            Verdict.INCONCLUSIVE,
            f"{spec.other_refusal}: {o.refusal.key} {o.refusal.message[:80]}",
        )
    if o.withheld:
        return Verdict.WITHHELD, f"the run's whole value was {WITHHELD_ANSWER}"
    if spec.lists_some and not o.covered:
        return Verdict.NOT_APPLICABLE, "not among the elements the answer lists"
    if not o.present:
        return Verdict.INCONCLUSIVE, "no presence proof around the probe"
    if not o.covered:
        return Verdict.INCONCLUSIVE, "the answer does not show the region"
    if o.slot is SlotState.OTHER:
        return Verdict.INCONCLUSIVE, "its slot holds something else"
    if o.slot is None and o.region.short:
        return Verdict.INCONCLUSIVE, "no slot to read a short value in"
    return Verdict.MASKED, "masked"


PIXEL_GRACE_MS = 5000  # a stream may deliver what it captured before `since` this late
# bringToFront closes every view before the private tab comes forward, and a closed view
# forwards nothing it reads after that (sdk-traces decision 46). So a view frame may trail
# the switch only by transport: one already in flight on its own socket as the reply left.
FRONT_GRACE_MS = 250


def decide_pixels(o: Observation) -> tuple[Verdict, str]:
    """Pixels matter only on a private tab and on a view display that should be paused
    (L3). There the contract is "no pixels at all", so the rule reads frame times only:

        a frame at or after the tab's privacy start   -> LEAKED (whatever it shows)
        a frame no record places against that start   -> INCONCLUSIVE
        a refusal (409, 410, page_private, no file)    -> WITHHELD
        tab never private                              -> N/A (BD-PIXELS: raw by design)
        frames before privacy and none after           -> MASKED (the stream was live)
        no frame before privacy and no refusal         -> INCONCLUSIVE (liveness never shown)
    """
    p = o.pixels
    if p and p.after:
        return Verdict.LEAKED, f"{p.after} frame(s) after the tab went private"
    if p and p.unplaced:
        return Verdict.INCONCLUSIVE, "a frame no record places before or after privacy"
    if o.refusal:
        return Verdict.WITHHELD, o.refusal.key
    if p is None or p.private_since is None:
        if o.channel is ChannelId.FRONT_VIEW:
            return Verdict.NOT_APPLICABLE, "the private tab never came to the front"
        return Verdict.NOT_APPLICABLE, "BD-PIXELS: the tab was not private"
    if not p.before:
        return Verdict.INCONCLUSIVE, "no frame before privacy: liveness never shown"
    return Verdict.MASKED, "stopped at privacy"


def decide_copy(o: Observation) -> tuple[Verdict, str]:
    """A copy or cut chord in a field (D6.3), by what the clipboard held after it. The
    field's value pasted back is a leak whatever the chord answered; anything short of a
    fresh baseline coming back proves nothing about the clipboard:

        a value in the run's answer                  -> LEAKED (as for every answer)
        the paste box holds the field's value        -> LEAKED (laundered)
        no presence proof before the chord           -> INCONCLUSIVE
        the field did not have the focus             -> INCONCLUSIVE
        the baseline copy refused, or not fresh      -> INCONCLUSIVE
        the paste box holds anything but it          -> INCONCLUSIVE
        the chord refused by contract                -> WITHHELD
        any other refusal                            -> INCONCLUSIVE
        the chord sent                               -> MASKED (sent, nothing copied)
    """
    clip = o.clip
    assert clip is not None
    if o.hit:
        return Verdict.LEAKED, f"{o.hit.form} value in the answer"
    if clip.pasted is Pasted.VALUE:
        return Verdict.LEAKED, "laundered: the paste box holds the field's value"
    if not o.present:
        return Verdict.INCONCLUSIVE, "no presence proof before the chord"
    if not clip.focused:
        return Verdict.INCONCLUSIVE, "the field did not have the focus before the chord"
    if not clip.baseline_sent:
        return (
            Verdict.INCONCLUSIVE,
            "the baseline copy was not sent: clipboard unproven",
        )
    if not clip.fresh:
        return (
            Verdict.INCONCLUSIVE,
            "the baseline token was not new: clipboard unproven",
        )
    if clip.pasted is not Pasted.BASELINE:
        return (
            Verdict.INCONCLUSIVE,
            f"the paste box holds {clip.pasted}: clipboard unproven",
        )
    if o.refusal is not None:
        if o.refusal.key in SPECS[o.channel].refusals:
            return Verdict.WITHHELD, f"{o.refusal.key}; the clipboard kept the baseline"
        return (
            Verdict.INCONCLUSIVE,
            f"refused: {o.refusal.key} {o.refusal.message[:80]}",
        )
    return Verdict.MASKED, "sent; the clipboard kept the baseline"


def decide_neo_upload(o: Observation) -> tuple[Verdict, str]:
    """A setInputFiles of one of Neo's own secret files (decision 33b). Neo must refuse
    it with DENY_CODE before reading a byte, and name no path; a directory holding a
    symlink to it is sent instead, as Chromium skips the symlink (m5-secrets). What the
    page read is the proof, whatever Neo answered:

        the page read a file that hashes as its target   -> LEAKED
        its target is not in this container              -> INCONCLUSIVE (no run)
        the harness could not make the links             -> N/A (no run)
        refused, the message names a path of it          -> LEAKED (the path)
        refused with DENY_CODE, or while it is held      -> WITHHELD
        cancelled with no file: a symlink directory
          from the background tab                        -> WITHHELD (Chromium's cancel)
        cancelled, where the deny list must refuse,
          or from the front tab                          -> INCONCLUSIVE
        refused any other way                            -> INCONCLUSIVE
        a symlink directory: the page read its decoy     -> MASKED (the symlink skipped)
        sent: no file, an unread or empty one, others    -> INCONCLUSIVE
    """
    n = o.neo
    assert n is not None
    if n.matched:
        return Verdict.LEAKED, f"{n.probe}: the page read its bytes"
    if not n.present:
        return Verdict.INCONCLUSIVE, f"{n.probe}: its target is not in this container"
    if not n.made:
        return (
            Verdict.NOT_APPLICABLE,
            f"{n.probe}: the harness could not make it ({n.why})",
        )
    if o.refusal is not None:
        key = o.refusal.key
        if n.names_path:
            return Verdict.LEAKED, f"{n.probe}: the refusal names its path ({key})"
        if key in SPECS[o.channel].refusals:
            return Verdict.WITHHELD, key
    if n.cancelled and not n.files:
        # Sent, and Chromium read nothing: its answer for a directory in a background
        # tab (setInputFiles then times out). Enough for a symlink directory there; not
        # for a probe the deny list must refuse, which only the background tab stopped.
        if n.refused:
            return (
                Verdict.INCONCLUSIVE,
                f"{n.probe}: cancelled, not refused: only the background tab kept "
                "the page from reading it",
            )
        if n.front:
            return (
                Verdict.INCONCLUSIVE,
                f"{n.probe}: cancelled in the front tab: nothing landed to show the "
                "symlink skipped",
            )
        return (
            Verdict.WITHHELD,
            "cancelled: Chromium reads no directory in a background tab",
        )
    if o.refusal is not None:
        key = o.refusal.key
        if not n.refused:
            return (
                Verdict.INCONCLUSIVE,
                f"{n.probe}: refused with {key}, not sent: {o.refusal.message[:80]}",
            )
        return (
            Verdict.INCONCLUSIVE,
            f"{n.probe}: refused with {key}, not {DENY_CODE}: {o.refusal.message[:80]}",
        )
    if not n.checked:
        return (
            Verdict.INCONCLUSIVE,
            f"{n.probe}: sent, but nothing checked what the page read",
        )
    if not n.refused and n.decoy:
        return (
            Verdict.MASKED,
            f"{n.probe}: sent; the page read the directory's own file, not the "
            "symlinked one",
        )
    if not n.files:
        return (
            Verdict.INCONCLUSIVE,
            f"{n.probe}: sent, but the page's input holds no file",
        )
    if n.empty:
        return (
            Verdict.INCONCLUSIVE,
            f"{n.probe}: sent, but it is empty: nothing to read back",
        )
    if not n.read:
        errors = ", ".join(n.errors) or "no reason given"
        return (
            Verdict.INCONCLUSIVE,
            f"{n.probe}: sent, but the page could not read it ({errors})",
        )
    return (
        Verdict.INCONCLUSIVE,
        f"{n.probe}: sent; the page read {n.read} of {n.files} files, none of them its "
        "bytes (it may have changed since)",
    )


# ------------------------------------------------------------------------------------------
# Observation: every (answer x region) worth a verdict
# ------------------------------------------------------------------------------------------

AGENT_FAMILIES = (
    Family.AGENT_TEXT,
    Family.AGENT_SCRIPT,
    Family.AGENT_PIXELS,
    Family.AGENT_KEYS,
)
DISK_CHANNELS: tuple[tuple[str, ChannelId], ...] = (
    ("claw-sdk/screenshots/", ChannelId.STORE_SCREENSHOTS),
    ("claw-sdk/trace.db", ChannelId.TRACE_STORE),
    ("logs/", ChannelId.CLAW_LOGS),
)
# Channels the judge can only search: no proof or coverage stands behind a cell there,
# so a search that finds nothing makes no cell. The table still shows their rows, as
# "clean" for each kind whose values the run proved present.
SEARCH_ONLY = frozenset(
    {
        ChannelId.HARNESS,
        ChannelId.HOST_REPLIES,
        ChannelId.CLIPBOARD_VIEW,
        ChannelId.CLIPBOARD_CONTROL,
        ChannelId.CLAW_LOGS,
        ChannelId.DOCKER_LOGS,
    }
)


class _Observer:
    """Builds the observations of one evidence directory. Holds the indexes the families
    share: the detector, each instance's privacy start, and which regions an agent
    capture proved present (records and disk inherit presence from those)."""

    def __init__(self, evidence: Evidence) -> None:
        self.ev = evidence
        self.detector = Detector(evidence.instances.values())
        self.offset = evidence.facts.clock_offset_ms if evidence.facts else 0
        self.private_since = self._private_since()
        self.front_since = self._front_since()
        self.present_regions: set[str] = set()
        self.present_runs: dict[str, set[str]] = {}  # run id -> regions it proved
        self.phase_of_run: dict[str, Phase] = {}
        self.captures = {c.id: c for c in evidence.captures}
        # run id -> the field box it typed: that run's records hold its typed text in
        # known places (typed_slots), which name no region, so the run attributes them.
        self.typed_runs: dict[str, Region] = {
            c.run_id: self.detector.regions[c.regions[0]]
            for c in evidence.captures
            if SPECS[c.channel].types and c.run_id and c.regions
        }
        self.bases: dict[str, object] = {}  # instance -> its last copy probe's baseline
        self.clips: dict[str, Clip] = {}  # copy probe capture id -> what it proved
        # channel -> [answers, bytes] searched: a channel with no cell was still read
        self.searched: dict[ChannelId, list[int]] = {}
        self.op_start = self._op_starts()
        self.hunted: set[str] = set()  # answers already hunted for Neo's own secrets
        # plant id -> the page showed the file an ordinary upload handed it
        self.received = {
            str(s.detail.get("plant")): bool(s.detail.get("shown"))
            for s in evidence.steps
            if s.action == "uploaded"
        }
        # A deny-list probe's step, by the run it records (neo_upload); one with no run
        # (a target missing here, a link not made) is judged from its step alone.
        probes = [s for s in evidence.steps if s.action == "neo_upload"]
        self.neo_runs = {
            str(s.detail["capture"]): s for s in probes if s.detail.get("capture")
        }
        self.neo_unrun = [s for s in probes if not s.detail.get("capture")]
        self.out: list[Observation] = []

    def _private_since(self) -> dict[str, int]:
        """Instance -> when its first tab went private, on the host clock. Neo's own
        `since` (no pixel at or after it is captured) comes on tab.private events; the
        step that asked for privacy is the fallback. Earliest wins."""
        tabs = {
            s.detail.get("tab"): s.instance
            for s in self.ev.steps
            if s.action == "open" and s.instance
        }
        since: dict[str, int] = {}

        def note(instance: str | None, at: int) -> None:
            if instance:
                since[instance] = min(at, since.get(instance, at))

        for step in self.ev.steps:
            if step.action in ("private", "takeover", "control_input"):
                note(step.instance, step.at_ms)
        for stream in self.ev.streams:
            if stream.channel is not ChannelId.EVENTS:
                continue
            for event in SseParser().feed(self.ev.blob(stream.blob)):
                if event.type != "tab.private":
                    continue
                with contextlib.suppress(ValueError, KeyError, TypeError):
                    data = json.loads(event.data)
                    at = int(data["privacy"]["since"]) - self.offset
                    note(tabs.get(data["tabId"]), at)
        return since

    def _front_since(self) -> dict[str, int]:
        """Instance -> when the agent's bringToFront put its private tab in front of the
        screen, on the host clock: where a view display must stop (decision 34). The
        harness records it as the op's end, read in front at once after it."""
        return {
            s.instance: int(s.detail["since_ms"])
            for s in self.ev.steps
            if s.action == "front"
            and s.instance
            and s.detail.get("private")
            and s.detail.get("in_front")
            and not s.detail.get("during")
        }

    def _op_starts(self) -> dict[str, tuple[int, str]]:
        """op id -> (start on the host clock, instance), from the run records fetched:
        operation screenshots carry no capture time of their own."""
        table: dict[str, tuple[int, str]] = {}
        for c in self.ev.captures:
            if c.channel is not ChannelId.RUN or not c.instance:
                continue
            try:
                run = json.loads(self.ev.blob(c.blob))
            except ValueError:
                continue
            for op in run.get("operations", ()):
                table[op["id"]] = (int(op["startedAt"]) - self.offset, c.instance)
        return table

    def emit(self, o: Observation) -> None:
        self.out.append(o)

    def _refusal(self, error: Mapping | None) -> Refusal | None:
        """A channel's "no" as it answered it. The message is the answer's own text, so
        it is redacted here, before a cell's reason quotes it cut short: a cut could leave
        less of a value than the redactor recognises."""
        if not error:
            return None
        message = self.detector.redact(str(error.get("message", "")))
        return Refusal(str(error.get("owner")), str(error.get("code")), message)

    def take(
        self, channel: ChannelId, source: str, data: bytes, run_id: str | None
    ) -> None:
        """Every answer the judge reads passes here first: tallied for the Searched
        table, and hunted once for Neo's own secrets (NEO_SECRETS). A hit is a new leak
        in this channel, its answer the evidence and its request the exact one. Only
        Neo's answers come here; the harness's own requests (whose headers carry the
        token by design) never do."""
        tally = self.searched.setdefault(channel, [0, 0])
        tally[0] += 1
        tally[1] += len(data)
        if source in self.hunted:
            return  # a view display's bytes are read for its pixels and its clipboard
        self.hunted.add(source)
        text = data.decode("utf-8", "replace")
        for secret in NEO_SECRETS:
            found = secret.pattern.search(text)
            if found is None:
                continue
            region = secret.subject
            context = self.detector.context(text, *found.span())
            hit = Hit(region.plant.id, region.id, Form.PLAIN, found.start(), context)
            self.emit(
                Observation(
                    channel,
                    region,
                    Phase.ANY,
                    source,
                    hit=hit,
                    present=None,
                    run_id=run_id,
                )
            )

    def pixel_regions(self, instance: Instance) -> list[Region]:
        """One proven field per kind: a frame shows the whole tab, so a pixel verdict is
        about the tab, and one region per kind carries it into the table."""
        seen: dict[Kind, Region] = {}
        for region in instance.regions:
            if region.exposure is Exposure.FIELD and region.id in self.present_regions:
                seen.setdefault(region.plant.kind, region)
        return list(seen.values())

    def hits_by_region(self, views: Views) -> dict[str, Hit]:
        """Every hit, keyed by its attributed region, or by the plant's first region:
        long values anywhere in the bytes, short ones in any region's anchored slot."""
        out: dict[str, Hit] = {}
        for hit in self.detector.hits(views.raw):
            plant = self.detector.plants[hit.plant]
            region = hit.region or next(
                r.id for r in self.detector.regions.values() if r.plant is plant
            )
            out.setdefault(region, hit)
        for hit in self.detector.slot_hits(views):
            out.setdefault(hit.region, hit)
        return out

    def agent(self, c: Capture) -> None:
        spec = SPECS[c.channel]
        data = self.ev.blob(c.blob)
        self.take(c.channel, c.id, data, c.run_id)
        reply = parse_run_reply(data)
        views = views_of(data)
        run_refusal = self._refusal(reply.error)
        before, after = (
            (self.ev.proofs.get(c.proofs[0]), self.ev.proofs.get(c.proofs[1]))
            if c.proofs
            else (None, None)
        )
        instance = self.ev.instances.get(c.instance or "")
        hits = self.hits_by_region(views)
        if c.run_id:
            self.phase_of_run[c.run_id] = c.phase
        is_pixel = spec.family is Family.AGENT_PIXELS
        since = self.private_since.get(c.instance or "")
        for region in instance.regions if instance else ():
            hit = hits.pop(region.id, None)
            live_before, live_after = live(before, region), live(after, region)
            # An op that answers nothing is judged on what it would expose, which the
            # page may show only once the op ran (setContent, an upload) or not at all.
            asked = spec.void and region.id in c.regions
            if not (live_before or live_after or hit or asked):
                continue
            present = (live_before or bool(c.trigger) or spec.types) and live_after
            if present:
                self.present_regions.add(region.id)
                if c.run_id:
                    self.present_runs.setdefault(c.run_id, set()).add(region.id)
            reads = views.keyed.get(region.id, ())
            refused = next((read for read in reads if "err" in read), None)
            pixels = None
            if is_pixel:
                private = since is not None and c.started_ms >= since
                got = reply.error is None
                pixels = Pixels(since if private else None, 0, int(private and got))
            self.emit(
                Observation(
                    c.channel,
                    region,
                    c.phase,
                    c.id,
                    hit=hit,
                    slot=self.detector.slot_state(views, region),
                    covered=self.detector.covered(views, region),
                    reachable=spec.reaches(region) and region.id in c.regions,
                    present=present,
                    refusal=run_refusal or self._refusal(refused),
                    pixels=pixels,
                    run_id=c.run_id,
                    received=self.received.get(region.plant.id, False),
                    withheld=reply.error is None and reply.value == WITHHELD_ANSWER,
                )
            )
        self._stray(c, hits)

    def _stray(self, c: Capture, hits: Mapping[str, Hit]) -> None:
        """A value where nobody asked is still a leak."""
        for region_id, hit in hits.items():
            self.emit(
                Observation(
                    c.channel,
                    self.detector.regions[region_id],
                    c.phase,
                    c.id,
                    hit=hit,
                    present=None,
                    run_id=c.run_id,
                )
            )

    def neo_upload(self, c: Capture) -> None:
        """A deny-list probe's run, judged by decide_neo_upload with its step's facts.
        The refusal's message is held against the probe's paths raw, before any
        redaction; the answer is searched for plants like any other."""
        data = self.ev.blob(c.blob)
        self.take(c.channel, c.id, data, c.run_id)
        reply = parse_run_reply(data)
        views = views_of(data)
        (region_id,) = c.regions
        keyed = (read for read in views.keyed.get(region_id, ()) if "err" in read)
        error = reply.error or next(keyed, None)
        refusal, names_path = None, False
        if error:
            # A run's own error says "message"; one void_run keyed by the input, "err".
            message = str(error.get("message") or error.get("err") or "")
            step = self.neo_runs.get(c.id)
            paths = step.detail.get("paths", ()) if step else ()
            names_path = any(path and str(path) in message for path in paths)
            owner, code = str(error.get("owner")), str(error.get("code"))
            refusal = Refusal(owner, code, self.detector.redact(message))
        self._neo(
            c.instance,
            region_id,
            c.id,
            self.neo_runs.get(c.id),
            c.run_id,
            refusal=refusal,
            names_path=names_path,
        )
        self._stray(c, self.hits_by_region(views))

    def _neo(
        self,
        instance: str | None,
        region_id: str,
        source: str,
        step: Step | None,
        run_id: str | None = None,
        *,
        refusal: Refusal | None = None,
        names_path: bool = False,
    ) -> None:
        """One deny-list probe's observation, from its step: a run whose step is missing
        was never checked, so its facts default to unproven."""
        region = self.ev.instances[instance or ""].region(region_id)
        d = step.detail if step else {}
        read = d.get("read")
        checked = isinstance(read, Mapping)
        read = read if checked else {}
        facts = NeoRead(
            probe=str(d.get("probe", region_id)),
            refused=bool(d.get("refused", True)),
            front=bool(d.get("front", False)),
            present=bool(d.get("present", True)),
            made=bool(d.get("made", True)),
            why=str(d.get("why") or ""),
            names_path=names_path,
            checked=checked,
            files=int(read.get("files", 0)),
            read=int(read.get("read", 0)),
            errors=tuple(str(e) for e in read.get("errors", ())),
            matched=bool(read.get("matched")),
            empty=bool(read.get("empty")),
            cancelled=bool(read.get("cancelled")),
            decoy=bool(read.get("decoy")),
        )
        self.emit(
            Observation(
                ChannelId.UPLOAD_NEO,
                region,
                Phase.ANY,
                source,
                reachable=True,
                refusal=refusal,
                neo=facts,
                run_id=run_id,
            )
        )

    def copy_probe(self, c: Capture) -> None:
        """A copy or cut chord in one field, its run's answer searched like any other.
        The chord's own answer is keyed by the field's id; the clipboard's is the paste
        box's digest in the proof after the run, held against the baseline token in the
        proof before it (the page made the token; it is no plant)."""
        data = self.ev.blob(c.blob)
        self.take(c.channel, c.id, data, c.run_id)
        views = views_of(data)
        hits = self.hits_by_region(views)
        reply = parse_run_reply(data)
        instance = self.ev.instances.get(c.instance or "")
        before, after = (
            (self.ev.proofs.get(c.proofs[0]), self.ev.proofs.get(c.proofs[1]))
            if c.proofs
            else (None, None)
        )
        answer = reply.value if isinstance(reply.value, dict) else {}
        chord = answer.get("chord") if isinstance(answer.get("chord"), dict) else {}
        refused = (
            {
                "owner": chord.get("owner"),
                "code": chord.get("code"),
                "message": chord["err"],
            }
            if "err" in chord
            else None
        )
        for region in [instance.region(r) for r in c.regions] if instance else []:
            clip = self._clip(c, region, before, after)
            self.clips[c.id] = clip
            if live(before, region):
                self.present_regions.add(region.id)
            self.emit(
                Observation(
                    c.channel,
                    region,
                    c.phase,
                    c.id,
                    hit=hits.pop(region.id, None),
                    covered=True,
                    reachable=True,
                    present=live(before, region),
                    refusal=self._refusal(reply.error) or self._refusal(refused),
                    clip=clip,
                    run_id=c.run_id,
                )
            )
        for region_id, hit in hits.items():  # a value where nobody asked is a leak
            self.emit(
                Observation(
                    c.channel,
                    self.detector.regions[region_id],
                    c.phase,
                    c.id,
                    hit=hit,
                    present=None,
                    run_id=c.run_id,
                )
            )

    def _clip(
        self, c: Capture, region: Region, before: Proof | None, after: Proof | None
    ) -> Clip:
        control = self.captures.get(c.control or "")
        sent = (
            control is not None
            and parse_run_reply(self.ev.blob(control.blob)).error is None
        )
        base = before.regions.get(COPY_BASE) if before else None
        drop = after.regions.get(COPY_DROP) if after else None
        fresh = base is not None and base != self.bases.get(c.instance or "")
        self.bases[c.instance or ""] = base
        expected = region.expected()
        if drop is None:
            pasted = Pasted.UNREAD
        elif drop == (fnv1a(expected), len(expected)):
            pasted = Pasted.VALUE
        elif drop == base:
            pasted = Pasted.BASELINE
        elif drop[1] == 0:
            pasted = Pasted.EMPTY
        else:
            pasted = Pasted.OTHER
        focused = before is not None and region.id in before.focus
        return Clip(sent, fresh, pasted, focused)

    def record(
        self, channel: ChannelId, source: str, data: bytes, run_id: str | None
    ) -> None:
        """A host record, an events payload, or a disk file: every value it holds is a
        hit. The whole answer is searched first; its run parts only attribute: each
        run's slice is judged where it shows a region that run's agent capture proved
        present (records inherit presence from their own run), and a value no run's
        slice holds (an event no run owns) is still a leak, with no run."""
        self.take(channel, source, data, run_id)
        disk = SPECS[channel].family is Family.DISK
        whole = views_of(data, structured=not disk)
        found = self.hits_by_region(whole)
        parts = [(run_id, whole)] if run_id or disk else self._run_parts(data)
        attributed: set[str] = set()  # plants some run's slice holds
        for part_run, views in parts or [(None, whole)]:
            hits = found if views is whole else self.hits_by_region(views)
            attributed |= {h.plant for h in hits.values()}
            present = (
                self.present_runs.get(part_run, set())
                if part_run
                else self.present_regions
            )
            phase = self.phase_of_run.get(part_run or "", Phase.ANY)
            # A typing run's part: what it typed, where its records keep it (the op's
            # typed argument, the code's literal), judged as that field's slots.
            typed = self.typed_runs.get(part_run or "")
            typed_states = (
                [classify_cut(s, typed) for s in typed_slots(views.raw)]
                if typed
                else []
            )
            for region_id in sorted(present | set(hits)):
                region = self.detector.regions[region_id]
                hit = hits.get(region_id)
                mine = region is typed and any(typed_states)
                if (
                    not hit
                    and not mine
                    and region.anchor not in views.raw
                    and region_id not in views.raw
                ):
                    continue  # the part never names this region
                slot = self.detector.slot_state(views, region)
                covered = self.detector.covered(views, region) or mine
                if mine:
                    slot = _worst([slot, *typed_states])
                if part_run is None and slot is SlotState.EMPTY:
                    slot = None  # with no run to inherit from, an empty slot proves nothing
                if disk:
                    # sqlite pages split strings: only a whole slot shows a region there
                    covered = slot is not None
                if not (hit or covered):
                    continue
                self.emit(
                    Observation(
                        channel,
                        region,
                        phase,
                        source,
                        hit=hit,
                        slot=slot,
                        covered=covered,
                        reachable=True,
                        present=region_id in present,
                        run_id=part_run,
                    )
                )
        for region_id, hit in found.items():
            if hit.plant not in attributed:
                region = self.detector.regions[region_id]
                self.emit(
                    Observation(
                        channel, region, Phase.ANY, source, hit=hit, present=None
                    )
                )

    def _run_parts(self, data: bytes) -> list[tuple[str, Views]]:
        """The record's slices by the run each belongs to: a list (runs, a trace tail,
        events) gives a part per run and per operation it holds. Bytes outside every
        slice (an envelope, an event no run owns) belong to no run."""
        parts: list[tuple[str, Views]] = []
        for tree in _json_trees(data.decode("utf-8", "replace")):
            for node in _leaves(tree):
                if not isinstance(node, dict):
                    continue
                owner = node.get("runId")
                if owner is None and ("operationCount" in node or "operations" in node):
                    owner = node.get("id")
                if isinstance(owner, str):
                    own = {k: v for k, v in node.items() if k != "operations"}
                    parts.append((owner, views_of(json.dumps(own).encode())))
        return parts

    def stream_pixels(self, s: Stream) -> None:
        """Screencast, video and view displays against the tab's privacy start."""
        instance = self.ev.instances.get(s.instance or "")
        if instance is None:
            return
        era = (
            self.front_since
            if s.channel is ChannelId.FRONT_VIEW
            else self.private_since
        )
        since = era.get(instance.name)
        grace = FRONT_GRACE_MS if s.channel is ChannelId.FRONT_VIEW else PIXEL_GRACE_MS
        refusal = Refusal("http", str(s.status)) if s.status not in (101, 200) else None
        frames = [] if refusal else self._frames(s)
        before = [a for a, _ in frames if since is None or a < since]
        after = [
            a
            for a, captured in frames
            if since is not None
            and ((captured is not None and captured >= since) or a > since + grace)
        ]
        if s.role == "retry":  # asked for after privacy: nothing is "before"
            before = []
        pixels = Pixels(since, len(before), len(after), after[0] if after else None)
        for region in self.pixel_regions(instance):
            self.emit(
                Observation(
                    s.channel,
                    region,
                    Phase.ANY,
                    s.id,
                    refusal=refusal,
                    pixels=pixels,
                    covered=True,
                    reachable=True,
                )
            )

    def _frames(self, s: Stream) -> list[tuple[int, int | None]]:
        """Every frame the stream delivered: (arrival, capture time when the stream
        states one), both on the host clock. Only the screencast states a capture time
        (X-Frame-At, on Neo's clock)."""
        data = self.ev.blob(s.blob)
        arrival = _arrival(s)
        if s.channel is ChannelId.SCREENCAST:
            return [
                (
                    arrival(p.offset),
                    None if p.frame_at is None else p.frame_at - self.offset,
                )
                for p in _replay(MjpegParser(), data, s)
            ]
        if s.channel is ChannelId.VIDEO:
            return [
                (arrival(o), None) for o, kind, _ in mp4_boxes(data) if kind == "moof"
            ]
        return [(arrival(m.offset), None) for m in _rfb(data, s) if m.type == 0]

    def clipboard(self, s: Stream) -> None:
        """Any ServerCutText is a leak: the image must never send the clipboard to an RFB
        client (-SendCutText=0 -SendPrimary=0). Each copy made while the display was open
        is judged against the cut texts that arrived in its window, from its start (a copy
        probe's `since_ms`; the stream's start for the first copy) to the next copy's. A
        cut text names the value it holds, or else the copied region. A copy whose value
        was proven on the clipboard (the owner's paste, a copy probe's pasted token) is
        the positive control that lets "no cut text" mean masked."""
        channel = (
            ChannelId.CLIPBOARD_CONTROL
            if s.role == "control"
            else ChannelId.CLIPBOARD_VIEW
        )
        data = self.ev.blob(s.blob)
        self.take(channel, s.id, data, None)
        copies = [
            st
            for st in self.ev.steps
            if st.action == "copy" and st.instance == s.instance
        ]
        pastes = [
            st
            for st in self.ev.steps
            if st.action == "paste" and st.instance == s.instance
        ]
        instance = self.ev.instances.get(s.instance or "")
        if not copies or instance is None or s.status != 101:
            return
        arrival = _arrival(s)
        cuts = [(arrival(m.offset), m) for m in _rfb(data, s) if m.type == 3]
        starts = [
            float("-inf") if n == 0 else float(st.detail.get("since_ms", st.at_ms))
            for n, st in enumerate(copies)
        ]
        ends = [*starts[1:], float("inf")]
        for copy, start, end in zip(copies, starts, ends, strict=True):
            region = instance.region(str(copy.detail["region"]))
            mine = [cut for at, cut in cuts if start <= at < end]
            found = [
                h
                for cut in mine
                for h in self.detector.hits((cut.text or b"").decode("latin-1"))
            ]
            hit = found[0] if found else None
            if mine and not hit:
                context = f"ServerCutText of {len(mine[0].text or b'')} bytes"
                hit = Hit(
                    region.plant.id, region.id, Form.SLOT, mine[0].offset, context
                )
            at = float(copy.detail.get("since_ms", copy.at_ms))
            open_at_copy = s.opened_ms <= at and (s.closed is None or s.closed[0] > at)
            probe = self.clips.get(str(copy.detail.get("capture")))
            if probe is not None:
                proven = probe.pasted in (Pasted.BASELINE, Pasted.VALUE)
            else:
                proven = any(p.detail.get("proven") for p in pastes)
            self.emit(
                Observation(
                    channel,
                    region,
                    Phase(copy.detail.get("phase", Phase.DURING_CONTROL)),
                    s.id,
                    hit=hit,
                    # No cut text at all: the channel carried nothing, a short value
                    # included, though nothing here is a slot to read it in.
                    slot=None if mine else SlotState.EMPTY,
                    covered=open_at_copy and proven,
                    reachable=open_at_copy,
                    present=proven,
                )
            )

    def op_screenshots(self, c: Capture) -> None:
        """GET /v1/operations/{op}/screenshot of an op on a tab that went private. The
        op's start and instance come from its run's record (a JPEG names neither)."""
        started, task = self.op_start.get(c.run_id or "", (None, None))
        name = c.instance or task or ""
        instance = self.ev.instances.get(name)
        since = self.private_since.get(name)
        if instance is None or since is None or started is None:
            return
        refusal = Refusal("http", str(c.status)) if c.status != 200 else None
        after = int(c.status == 200 and started >= since)
        pixels = Pixels(
            since if started >= since else None, int(started < since), after
        )
        for region in self.pixel_regions(instance):
            self.emit(
                Observation(
                    ChannelId.OP_SCREENSHOT,
                    region,
                    Phase.ANY,
                    c.id,
                    refusal=refusal,
                    pixels=pixels,
                    covered=True,
                    reachable=True,
                )
            )

    def run(self) -> list[Observation]:
        agent = [
            c for c in self.ev.captures if SPECS[c.channel].family in AGENT_FAMILIES
        ]
        for c in agent:
            if SPECS[c.channel].family is Family.AGENT_KEYS:
                self.copy_probe(c)
            elif c.channel is ChannelId.UPLOAD_NEO:
                self.neo_upload(c)
            elif c.channel is not ChannelId.REPLAY:
                self.agent(c)
        for step in self.neo_unrun:
            self._neo(step.instance, str(step.detail["region"]), "harness", step)
        for c in agent:
            if c.channel is ChannelId.REPLAY:
                self.record(ChannelId.REPLAY, c.id, self.ev.blob(c.blob), c.run_id)
        for c in self.ev.captures:
            family = SPECS[c.channel].family
            if c.channel is ChannelId.OP_SCREENSHOT:
                self.op_screenshots(c)
            elif family in (Family.HOST_RECORD, Family.HOST_REPLY):
                self.record(c.channel, c.id, self.ev.blob(c.blob), c.run_id)
        for s in self.ev.streams:
            if s.channel in (ChannelId.EVENTS, ChannelId.EVENTS_REPLAY):
                self.record(s.channel, s.id, self.ev.blob(s.blob), None)
            elif s.channel is ChannelId.DISPLAY_VIEW and s.role == "control":
                self.clipboard(s)
            elif SPECS[s.channel].family is Family.LIVE_PIXELS:
                self.stream_pixels(s)
                # The only text a pixel stream carries (the screencast's X-Url) is
                # searched like any other answer.
                self.record(s.channel, s.id, self.ev.blob(s.blob), None)
                if s.channel is ChannelId.DISPLAY_VIEW:
                    self.clipboard(s)
        for path in sorted((self.ev.root / "disk").rglob("*")):
            rel = path.relative_to(self.ev.root / "disk").as_posix()
            channel = next((ch for prefix, ch in DISK_CHANNELS if prefix in rel), None)
            if (
                path.is_file()
                and channel
                and channel is not ChannelId.STORE_SCREENSHOTS
            ):
                self.record(channel, f"disk/{rel}", path.read_bytes(), None)
        self.store_screenshots()
        docker_log = self.ev.root / "docker.log"
        if docker_log.exists():
            self.record(
                ChannelId.DOCKER_LOGS, "docker.log", docker_log.read_bytes(), None
            )
        for step in self.ev.steps:
            if step.action == "profile_scan":
                for hit in step.detail.get("hits", ()):
                    region = next(
                        r
                        for r in self.detector.regions.values()
                        if r.plant.id == hit["plant"]
                    )
                    self.emit(
                        Observation(
                            ChannelId.PROFILE,
                            region,
                            Phase.ANY,
                            hit["file"],
                            hit=Hit(hit["plant"], None, Form.PLAIN, 0, hit["file"]),
                            present=None,
                        )
                    )
        return self.out

    def store_screenshots(self) -> None:
        """screenshots/<run>/<seq>.jpg in each disk copy, against the ops the run records
        place. A file of an op that ran on a private tab at or after its privacy start is
        a pixel leak; such an op that left no file in any copy is held (absence counts
        only where a copy of the state dir was taken); a file no record places could be
        a private tab's, so it is unproven, never dropped."""
        disk = self.ev.root / "disk"
        copies = [d for d in sorted(disk.glob("*")) if (d / "claw-sdk").is_dir()]
        files = sorted(
            path
            for copy in copies
            for path in (copy / "claw-sdk/screenshots").glob("*/*")
            if path.is_file()
        )
        for path in files:
            source = f"disk/{path.relative_to(disk).as_posix()}"
            self._screenshot(f"{path.parent.name}.{path.stem}", source, found=True)
        if copies:
            shot = {f"{path.parent.name}.{path.stem}" for path in files}
            for op in sorted(self.op_start.keys() - shot):
                self._screenshot(op, f"no file for op {op}", found=False)

    def _screenshot(self, op: str, source: str, *, found: bool) -> None:
        started, name = self.op_start.get(op, (None, None))
        if started is None:  # a file whose op no run record lists
            run = op.rpartition(".")[0]
            owners = {
                i for o, (_, i) in self.op_start.items() if o.startswith(run + ".")
            }
            for name in sorted(owners or self.private_since.keys()):
                if name in self.private_since:
                    unplaced = Pixels(self.private_since[name], 0, 0, unplaced=1)
                    self._emit_pixels(name, source, unplaced)
            return
        since = self.private_since.get(name)
        if since is None or (not found and started < since):
            return  # a tab never private is raw by design; no file before privacy is moot
        if found:
            before, after = int(started < since), int(started >= since)
            self._emit_pixels(
                name, source, Pixels(since if after else None, before, after)
            )
        else:
            no_file = Refusal("disk", "no_file")
            self._emit_pixels(name, source, Pixels(since, 0, 0), no_file)

    def _emit_pixels(
        self, name: str, source: str, pixels: Pixels, refusal: Refusal | None = None
    ) -> None:
        for region in self.pixel_regions(self.ev.instances[name]):
            self.emit(
                Observation(
                    ChannelId.STORE_SCREENSHOTS,
                    region,
                    Phase.ANY,
                    source,
                    refusal=refusal,
                    pixels=pixels,
                    covered=True,
                    reachable=True,
                )
            )


def _replay(parser: MjpegParser | RfbParser, data: bytes, s: Stream) -> list:
    """A recorded stream through its parser, chunk by chunk as it arrived. Bytes a parser
    cannot frame end the replay there and keep what came before: the stream itself ended
    at that point too (the reader thread used the same parser)."""
    out: list = []
    bounds = [o for o, _ in s.chunks] + [len(data)]
    for start, end in zip(bounds, bounds[1:], strict=False):
        try:
            out += parser.feed(data[start:end])
        except ValueError:
            break
    return out


def _rfb(data: bytes, s: Stream) -> list:
    width, height, fmt = s.geometry or (0, 0, "")
    return _replay(RfbParser(width, height, bytes.fromhex(fmt)), data, s)


def _arrival(s: Stream) -> Callable[[int], int]:
    """When the byte at a stream offset arrived, on the host clock: its chunk's time."""
    offsets = [o for o, _ in s.chunks]
    times = [t for _, t in s.chunks]

    def arrival(offset: int) -> int:
        return times[max(0, bisect.bisect_right(offsets, offset) - 1)] if times else 0

    return arrival


def observe(evidence: Evidence) -> list[Observation]:
    return _Observer(evidence).run()


# ------------------------------------------------------------------------------------------
# Residuals: known leaks, labelled apart from new ones
# ------------------------------------------------------------------------------------------


class Status(StrEnum):
    IN_FLIGHT = "in flight"  # a fix is on its way; the residual should disappear
    BY_DESIGN = "by design"  # the design accepts it; changing that is the owner's call
    CONTROL = "control"  # the expected answer of a control: never sets an exit bit


@dataclasses.dataclass(frozen=True)
class Cell:
    """A judged (channel, region, phase): the atom of the report."""

    id: str
    channel: ChannelId
    region: Region
    phase: Phase
    verdict: Verdict
    why: str
    source: str
    run_id: str | None = None
    hit: Hit | None = None
    residual: str | None = None
    derived_from: str | None = None


def _cold_text_leak(cell: Cell) -> bool:
    return (
        cell.verdict is Verdict.LEAKED
        and SPECS[cell.channel].family is Family.AGENT_TEXT
        and cell.phase in (Phase.COLD, Phase.JOURNALED)
    )


def is_fx_f(cell: Cell) -> bool:
    """A cold agent read of an echo of a same-site plant in another frame: the read
    masked with less than its frame's whole site."""
    loc = cell.region.plant.location
    return (
        _cold_text_leak(cell)
        and cell.region.exposure in ECHO_LIKE
        and loc not in (Location.MAIN, Location.SHADOW_OPEN, Location.SHADOW_CLOSED)
        and PLACES[loc].same_site_as_top
    )


def is_xsite(cell: Cell) -> bool:
    """A cold read of a cross-site plant the page copied into the top document: no
    sweep of the top's site sees the source frame, so only L2 can catch it. Main-world
    script reading that echo meets the same gap: its guard sweeps the page's site."""
    return (
        cell.verdict is Verdict.LEAKED
        and SPECS[cell.channel].family in (Family.AGENT_TEXT, Family.AGENT_SCRIPT)
        and cell.phase is Phase.COLD
        and cell.region.exposure in ECHO_LIKE
        and not PLACES[cell.region.plant.location].same_site_as_top
    )


def is_fx_f_hint(cell: Cell) -> bool:
    """A strict-mode error's locator hint carrying part of a value: Playwright cuts the
    hint text before Rust masks whole values (named "known, next" in 019df89eb)."""
    return (
        cell.verdict is Verdict.LEAKED
        and cell.channel is ChannelId.STRICT_PREVIEW
        and cell.hit is not None
        and cell.hit.form is Form.FRAGMENT
    )


def is_eval_held(cell: Cell) -> bool:
    return (
        cell.verdict is Verdict.WITHHELD
        and SPECS[cell.channel].family is Family.AGENT_SCRIPT
    )


def is_short_echo(cell: Cell) -> bool:
    return (
        cell.verdict is Verdict.LEAKED
        and cell.region.exposure in ECHO_LIKE
        and cell.region.short
    )


def is_l2_only(cell: Cell) -> bool:
    """A cold leak through an exit the design masks with L2 only, or of a plant in
    another tab or in no field (a host value before the host registered it). Not a
    main-world script's read of another tab: the guard sweeps every tab an opener link
    connects to the script's page (sdk-playwright M3), so that one is new."""
    plant = cell.region.plant
    other_tab = PLACES[cell.region.location].other_tab
    return (
        cell.verdict is Verdict.LEAKED
        and cell.phase is Phase.COLD
        and (
            SPECS[cell.channel].l2_only
            or plant.origin is Origin.HOST
            or (other_tab and SPECS[cell.channel].family is not Family.AGENT_SCRIPT)
        )
    )


def _kind_is(kind: Kind) -> Callable[[Cell], bool]:
    return (
        lambda cell: cell.verdict is Verdict.LEAKED and cell.region.plant.kind is kind
    )


@dataclasses.dataclass(frozen=True)
class Residual:
    id: str
    status: Status
    why: str  # printed in the report beside every cell it labels
    applies: Callable[[Cell], bool]


# The one list of known residuals. The brief names the first two; the rest are what the
# design itself accepts. A leak that matches nothing here is new and sets EXIT_NEW_LEAK.
RESIDUALS: tuple[Residual, ...] = (
    Residual(
        # Not on the clipboard channels: any ServerCutText there is the flag failing,
        # whatever value it carries, so a control's value there is a finding too. The
        # agent's own upload buffer is one: a file input is a field no rule classifies.
        "CONTROL",
        Status.CONTROL,
        "a field no rule classifies, read back raw: the positive control",
        lambda cell: (
            (
                _kind_is(Kind.PLAIN)(cell)
                or (
                    _kind_is(Kind.FILE)(cell)
                    and cell.region.plant.origin is Origin.AGENT
                )
            )
            and SPECS[cell.channel].family is not Family.CLIPBOARD
        ),
    ),
    Residual(
        # The agent's read only: a record copies it by run, so Neo's own record of the
        # upload op holding the file is a new leak.
        "BD-UPLOAD",
        Status.BY_DESIGN,
        "a file from Neo's disk the agent uploaded and read back from the page: no rule "
        "can tell its content is sensitive, and setInputFiles takes any path (owner "
        "decision: the caller sandboxes)",
        lambda cell: (
            _kind_is(Kind.FILE)(cell)
            and cell.region.plant.origin in (Origin.FILE, Origin.DOWNLOAD)
            and SPECS[cell.channel].family in AGENT_FAMILIES
        ),
    ),
    Residual(
        # Held whole, not refused by the deny list: DENY_CODE is the contract working.
        "UPLOAD-HELD",
        Status.IN_FLIGHT,
        "setInputFiles is held (script/method_unavailable) until the owner's call to "
        "serve any path lands",
        lambda cell: (
            cell.verdict is Verdict.WITHHELD
            and cell.channel
            in (ChannelId.UPLOAD, ChannelId.UPLOAD_BUFFER, ChannelId.UPLOAD_NEO)
            and cell.why in HELD
        ),
    ),
    Residual(
        "BD-R1",
        Status.BY_DESIGN,
        "a field no rule classifies (risk R1)",
        _kind_is(Kind.GAP_UNLABELLED),
    ),
    Residual(
        "BD-R3",
        Status.BY_DESIGN,
        "a password shown as text before Neo's first touch (risk R3)",
        _kind_is(Kind.GAP_EARLY_TOGGLE),
    ),
    Residual(
        "BD-SHORT",
        Status.BY_DESIGN,
        "L2 masks values of 4+ characters only, and a split code's boxes hold one digit "
        "each, so no sweep learns the joined code (D1)",
        is_short_echo,
    ),
    Residual(
        "FX-F",
        Status.IN_FLIGHT,
        "a read masked with less than its frame's whole site (sdk-playwright FX-F)",
        is_fx_f,
    ),
    Residual(
        "FX-F-HINT",
        Status.IN_FLIGHT,
        "a strict-mode hint is cut before the whole-value mask, so a value whose spelling "
        "the cut splits leaks its head (019df89eb: mask before the trim, queued)",
        is_fx_f_hint,
    ),
    Residual(
        "EVAL-HELD",
        Status.IN_FLIGHT,
        "main-world script is held (script/method_unavailable) until its guard is served",
        is_eval_held,
    ),
    Residual(
        "BD-L2",
        Status.BY_DESIGN,
        "an exit the design masks with L2 only, or a value in another tab or in no field, "
        "read before Neo learned it (D1)",
        is_l2_only,
    ),
    Residual(
        "XSITE",
        Status.BY_DESIGN,
        "a cross-site frame posted its value into the top document: the site sweep never "
        "sees that frame, so only L2 can mask it (decided overnight; a recommendation)",
        is_xsite,
    ),
    Residual(
        "BD-PROFILE",
        Status.BY_DESIGN,
        "Chromium's own profile state",
        lambda cell: cell.channel is ChannelId.PROFILE,
    ),
)
RESIDUAL_STATUS = {r.id: r.status for r in RESIDUALS}


def assign_residuals(cells: Sequence[Cell]) -> list[Cell]:
    """Label leaks (and held answers) with the residual that explains them.

    A record or disk copy of a leak takes the residual of the agent-side leak of the same
    plant through the same run (`derived_from`). A copy that names its run inherits from
    that run only: when the run's own answer did not leak the plant, the copy is a new
    leak, whatever other runs leaked. When it did, the copy takes that answer's label even
    over a rule that labels the copy itself, since a copy of a new leak is new (a record
    of a script's read of another tab would otherwise read as BD-L2). One with no run
    takes a residual only when every agent-side leak of that plant carries the same one,
    and keeps a rule's label; otherwise it is new. Without this, one cold FX-F read would
    show up as six new host leaks.
    """
    out: list[Cell] = []
    agent_leaks: dict[str, list[Cell]] = {}
    for cell in cells:
        residual = next((r.id for r in RESIDUALS if r.applies(cell)), None)
        labelled = dataclasses.replace(cell, residual=residual)
        out.append(labelled)
        if (
            cell.verdict is Verdict.LEAKED
            and SPECS[cell.channel].family in AGENT_FAMILIES
            and cell.channel is not ChannelId.REPLAY
        ):
            agent_leaks.setdefault(cell.region.plant.id, []).append(labelled)
    for i, cell in enumerate(out):
        if cell.verdict is not Verdict.LEAKED:
            continue
        if (
            SPECS[cell.channel].family in AGENT_FAMILIES
            and cell.channel is not ChannelId.REPLAY
        ):
            continue
        sources = agent_leaks.get(cell.region.plant.id, [])
        if cell.run_id:
            pool = [s for s in sources if s.run_id == cell.run_id]
        elif cell.residual is None:
            pool = sources
        else:
            continue
        labels = {s.residual for s in pool}
        if pool and len(labels) == 1:
            out[i] = dataclasses.replace(
                cell, residual=labels.pop(), derived_from=pool[0].id
            )
    return out


# ------------------------------------------------------------------------------------------
# The judgement, the report and the exit status
# ------------------------------------------------------------------------------------------

EXIT_NEW_LEAK = 1
EXIT_KNOWN = 4  # a known residual was observed: leaked or held
EXIT_INCONCLUSIVE = 8
EXIT_HARNESS = 16  # a scenario raised or preflight failed: the table is incomplete


def cell_rank(cell: Cell) -> int:
    """Worst first: a table cell shows the worst verdict among its observations."""
    status = RESIDUAL_STATUS.get(cell.residual or "")
    if cell.verdict is Verdict.LEAKED and status is None:
        return 0
    if cell.verdict is Verdict.INCONCLUSIVE:
        return 1
    if cell.verdict is Verdict.LEAKED and status is Status.IN_FLIGHT:
        return 2
    if cell.verdict is Verdict.LEAKED and status is Status.BY_DESIGN:
        return 3
    if cell.verdict is Verdict.WITHHELD and status is Status.IN_FLIGHT:
        return 4
    if cell.verdict is Verdict.MASKED:
        return 5
    if cell.verdict is Verdict.WITHHELD:
        return 6
    if cell.verdict is Verdict.LEAKED:  # a control's expected raw read
        return 7
    return 8


@dataclasses.dataclass(frozen=True)
class Judgement:
    cells: tuple[Cell, ...]
    failures: tuple[str, ...]
    headline: Mapping[str, str]  # FX-E, FX-F, XSITE, EVAL-HELD: what was measured
    searched: Mapping[ChannelId, tuple[int, int]] = dataclasses.field(
        default_factory=dict
    )  # channel -> (answers, bytes) searched for every planted value
    present: frozenset[Kind] = frozenset()  # kinds some oracle proof showed live

    @property
    def exit_code(self) -> int:
        code = 0
        for cell in self.cells:
            status = RESIDUAL_STATUS.get(cell.residual or "")
            if cell.verdict is Verdict.LEAKED and status is None:
                code |= EXIT_NEW_LEAK
            elif status in (Status.IN_FLIGHT, Status.BY_DESIGN) and cell.verdict in (
                Verdict.LEAKED,
                Verdict.WITHHELD,
            ):
                code |= EXIT_KNOWN
            elif cell.verdict is Verdict.INCONCLUSIVE:
                code |= EXIT_INCONCLUSIVE
        return code | (EXIT_HARNESS if self.failures else 0)


def _tally(found: Sequence[Cell]) -> str:
    """How many of these masked-or-leaked reads came back raw."""
    if not found:
        return "not measured"
    leaked = [c for c in found if c.verdict is Verdict.LEAKED]
    if leaked:
        return f"raw in {len(leaked)} of {len(found)} reads"
    return f"masked in all {len(found)} reads"


def _judged(cell: Cell) -> bool:
    """A read of a long sensitive value that came back masked or raw: what a headline
    counts (short echoes are by design, the controls raw by definition)."""
    return (
        KINDS[cell.region.plant.kind].sensitive
        and not cell.region.short
        and cell.verdict in (Verdict.MASKED, Verdict.LEAKED)
    )


def _headline(cells: Sequence[Cell]) -> dict[str, str]:
    """What the cold echo reads measured, by where the echoed field sits; what the
    main-world family and setContent answered."""

    def cold_echoes(locations: Collection[Location]) -> str:
        return _tally(
            [
                c
                for c in cells
                if c.phase is Phase.COLD
                and c.region.exposure is Exposure.ECHO
                and SPECS[c.channel].family is Family.AGENT_TEXT
                and c.region.plant.location in locations
                and c.region.plant.origin is not Origin.HOST
                and _judged(c)
            ]
        )

    content_held = any(
        c.channel is ChannelId.SET_CONTENT and c.verdict is Verdict.WITHHELD
        for c in cells
    )
    content = (
        "held (script/method_unavailable)"
        if content_held
        else _tally([c for c in cells if c.phase is Phase.AFTER_CONTENT and _judged(c)])
    )

    held = [c for c in cells if c.residual == "EVAL-HELD"]
    served = [
        c
        for c in cells
        if SPECS[c.channel].family is Family.AGENT_SCRIPT and _judged(c)
    ]
    raw = collections.Counter(
        c.residual or "new" for c in served if c.verdict is Verdict.LEAKED
    )
    script = [
        f"held in {len({c.channel for c in held})} channels (script/method_unavailable)"
        if held
        else "",
        f"served: {_tally(served)}"
        + (f" ({', '.join(f'{n} {k}' for k, n in sorted(raw.items()))})" if raw else "")
        if served
        else "",
    ]
    chords = [
        c
        for c in cells
        if SPECS[c.channel].family is Family.AGENT_KEYS
        and KINDS[c.region.plant.kind].sensitive
    ]
    tally = {verdict: sum(c.verdict is verdict for c in chords) for verdict in Verdict}
    return {
        "copy and cut chords in sensitive fields (D6.3)": (
            f"{tally[Verdict.WITHHELD]} refused, {tally[Verdict.MASKED]} sent with "
            f"nothing copied, {tally[Verdict.LEAKED]} laundered or leaked, "
            f"{tally[Verdict.INCONCLUSIVE]} unproven, of {len(chords)}"
            if chords
            else "not measured"
        ),
        "FX-E same-document echo, cold (M, SH)": cold_echoes(
            (Location.MAIN, Location.SHADOW_OPEN)
        ),
        "FX-F same-site cross-frame echo, cold (SO, SS, XN)": cold_echoes(
            (Location.SAME_ORIGIN, Location.SAME_SITE, Location.NESTED)
        ),
        "XSITE cross-site echo, cold (XO)": cold_echoes((Location.CROSS_SITE,)),
        "closed shadow root echo, cold (SC)": cold_echoes((Location.SHADOW_CLOSED,)),
        "main-world script": "; ".join(p for p in script if p) or "not measured",
        "page.setContent, then reads of what it wrote": content,
    }


def judge(evidence: Evidence) -> Judgement:
    """observe -> decide (decide_pixels for pixel channels) -> assign_residuals."""
    observer = _Observer(evidence)
    cells = []
    for n, o in enumerate(observer.run()):
        if o.pixels is not None:
            verdict, why = decide_pixels(o)
        elif o.clip is not None:
            verdict, why = decide_copy(o)
        elif o.neo is not None:
            verdict, why = decide_neo_upload(o)
        else:
            verdict, why = decide(o)
        cells.append(
            Cell(
                f"{o.source}:{o.region.id}:{n}",
                o.channel,
                o.region,
                o.phase,
                verdict,
                why,
                o.source,
                o.run_id,
                o.hit,
            )
        )
    evaluations = _evaluations(evidence)
    labelled = assign_residuals(_sent_refusals(cells, evaluations))
    searched = {k: (v[0], v[1]) for k, v in observer.searched.items()}
    headline = _headline(labelled)
    headline.update(_opener_links(labelled, evaluations))
    captures = {c.id: c for c in evidence.captures}
    replays = [c for c in evidence.captures if c.replay_of in captures]
    if replays:
        same = sum(
            parse_run_reply(evidence.blob(c.blob)).value
            == parse_run_reply(evidence.blob(captures[c.replay_of].blob)).value
            for c in replays
        )
        headline["requestId replay returns the stored reply"] = (
            f"{same} of {len(replays)}"
        )
    headline["setInputFiles of Neo's own files (decision 33b)"] = _neo_uploads(
        labelled, evidence.steps
    )
    headline.update(_fronts(evidence.steps))
    present = frozenset(
        region.plant.kind
        for proof in evidence.proofs.values()
        if proof.instance in evidence.instances
        for region in evidence.instances[proof.instance].regions
        if live(proof, region)
    )
    return Judgement(tuple(labelled), evidence.failures, headline, searched, present)


def _evaluations(evidence: Evidence) -> dict[str, list[str]]:
    """Each run's page.evaluate ops by their delivery ("not_sent", "done", "unknown"), as
    the host's operation records state it. A run refused before any op has none."""
    out: dict[str, list[str]] = {}
    for c in evidence.captures:
        if c.channel is not ChannelId.OPERATION or not c.run_id:
            continue
        try:
            record = json.loads(evidence.blob(c.blob))
        except (OSError, ValueError):
            continue
        if isinstance(record, dict) and record.get("name") == "page.evaluate":
            out.setdefault(c.run_id, []).append(str(record.get("delivery")))
    return out


def _sent_refusals(
    cells: Sequence[Cell], evaluations: Mapping[str, Sequence[str]]
) -> list[Cell]:
    """An opener probe's refusal holds only with nothing sent: once its run's script
    reached a page (delivery done or unknown), the refusal no longer shows the other tab
    stayed untouched, so its cells are unproven."""
    out = []
    for cell in cells:
        sent = sorted(
            {d for d in evaluations.get(cell.run_id or "", ()) if d != "not_sent"}
        )
        if cell.channel in OPENER_PROBES and cell.verdict is Verdict.WITHHELD and sent:
            why = (
                f"refused ({cell.why}) after page.evaluate was sent ({', '.join(sent)})"
            )
            cell = dataclasses.replace(cell, verdict=Verdict.INCONCLUSIVE, why=why)
        out.append(cell)
    return out


def _opener_links(
    cells: Sequence[Cell], evaluations: Mapping[str, Sequence[str]]
) -> dict[str, str]:
    """Each main-world read across an opener link (finding 6) in one line.

    Refused, it is closed only when nothing was sent and none of the probe page's values
    turns up raw in any exit: the reply and its error, every /v1 record and event,
    trace.db. A refused run returns nothing, so even a by-design residual found raw
    reopens it. Served, it is what its reads came back as, new leaks apart from the known
    residuals (the PLAIN control and the R1 and R3 gaps leave raw by design)."""
    out = {}
    for channel in OPENER_PROBES:
        key = f"{channel} across the opener link (finding 6)"
        agent = [c for c in cells if c.channel is channel]
        judged = [c for c in agent if c.verdict is not Verdict.NOT_APPLICABLE]
        if not judged:
            out[key] = "not measured"
            continue
        plants = {c.region.plant.id for c in agent}
        raw = [
            c
            for c in cells
            if c.verdict is Verdict.LEAKED and c.region.plant.id in plants
        ]
        refused = {c.why for c in judged if c.verdict is Verdict.WITHHELD}
        if all(c.verdict is Verdict.WITHHELD for c in judged):
            sent = sorted(
                {d for c in judged for d in evaluations.get(c.run_id or "", ())}
            )
            delivery = f"page.evaluate {', '.join(sent)}" if sent else "no op"
            found = f"LEAK: {len(raw)} cells raw" if raw else "nothing raw in any exit"
            out[key] = f"refused ({', '.join(sorted(refused))}; {delivery}), {found}"
            continue
        new = [c for c in raw if c.residual is None]
        known = collections.Counter(c.residual for c in raw if c.residual)
        verdicts = collections.Counter(c.verdict for c in judged)
        parts = [f"{len(new)} new leaks" if new else "no new leak"]
        parts += [
            f"{verdicts[v]} {v}"
            for v in (Verdict.MASKED, Verdict.WITHHELD, Verdict.INCONCLUSIVE)
            if verdicts[v]
        ]
        if known:
            parts.append(
                "raw by design: "
                + ", ".join(f"{n} {k}" for k, n in sorted(known.items()))
            )
        out[key] = "served: " + "; ".join(parts)
    return out


def _neo_uploads(cells: Sequence[Cell], steps: Sequence[Step]) -> str:
    """The deny-list probes in one line: held by which refusal, every probe that leaked
    or stayed unproven by its reason, and whether the read-back's control passed."""
    probes = [
        c
        for c in cells
        if c.channel is ChannelId.UPLOAD_NEO and c.region.plant.origin is Origin.NEO
    ]
    if not probes:
        return "not measured"
    held = collections.Counter(c.why for c in probes if c.verdict is Verdict.WITHHELD)
    contract_first = sorted(held.items(), key=lambda kv: (kv[0] != DENY_CODE, kv[0]))
    parts = [f"{n} held ({key})" for key, n in contract_first]
    for verdict, word in (
        (Verdict.MASKED, "masked"),
        (Verdict.LEAKED, "LEAK"),
        (Verdict.INCONCLUSIVE, "unproven"),
        (Verdict.NOT_APPLICABLE, "not run"),
    ):
        whys = [c.why for c in probes if c.verdict is verdict]
        if whys:
            parts.append(f"{len(whys)} {word} ({'; '.join(whys)})")
    controls = [s.detail for s in steps if s.action == "neo_upload_control"]
    control = (
        "not run" if not controls else "ok" if controls[-1].get("matched") else "FAILED"
    )
    return f"{len(probes)} probes: {', '.join(parts)}; read-back control {control}"


def _fronts(steps: Sequence[Step]) -> dict[str, str]:
    """What the agent's bringToFront did, as the harness read the front tab around it:
    onto a private tab (the view rows judge the pixels), and during a takeover, whose
    tab must stay in front unless the call was refused."""
    out = {
        "page.bringToFront onto a private tab": "not measured",
        "page.bringToFront during a takeover": "not measured",
    }
    for step in steps:
        if step.action != "front":
            continue
        d = step.detail
        answer = str(d.get("answer"))
        if d.get("during"):
            stayed = d["during"] in d.get("after", ())
            out["page.bringToFront during a takeover"] = (
                f"{answer}; the takeover's tab "
                + ("stayed in front" if stayed else "left the front")
            )
        else:
            came = "came" if d.get("in_front") else "did not come"
            out["page.bringToFront onto a private tab"] = (
                f"sent; the private tab {came} to the front"
                if answer == "sent"
                else answer
            )
    return out


_GLYPH = {0: "LEAK", 1: "??", 4: "held", 5: "ok", 6: "held", 7: "raw", 8: "·"}


def glyph(cell: Cell) -> str:
    rank = cell_rank(cell)
    if cell.verdict is Verdict.NOT_APPLICABLE and cell.why.startswith("not built"):
        return "nb"
    if rank in (2, 3):
        return (cell.residual or "").lower().replace("bd-", "")
    return _GLYPH[rank]


def table(judgement: Judgement) -> dict[ChannelId, dict[Kind, Cell]]:
    """channel x kind -> the worst cell among its observations."""
    out: dict[ChannelId, dict[Kind, Cell]] = {}
    for cell in judgement.cells:
        row = out.setdefault(cell.channel, {})
        kind = cell.region.plant.kind
        if kind not in row or cell_rank(cell) < cell_rank(row[kind]):
            row[kind] = cell
    return out


def _tags(cells: Sequence[Cell], worst: Cell) -> str:
    same = [
        c
        for c in cells
        if cell_rank(c) == cell_rank(worst) and c.residual == worst.residual
    ]
    places = sorted({PLACES[c.region.plant.location].code for c in same})
    phases = sorted({str(c.phase) for c in same if c.phase is not Phase.ANY})
    tags = ",".join(places[:3]) + ("…" if len(places) > 3 else "")
    return f"{tags}·{'/'.join(phases[:2])}" if phases else tags


def render_markdown(judgement: Judgement, evidence: Evidence) -> str:
    """report.md: facts, the headline, the channel x kind table, every new leak with its
    exact request, residuals, inconclusive cells by reason. The whole text passes the
    run's redactor, so no diagnostic (a failure, a refusal, a fact) prints a value."""
    detector = Detector(evidence.instances.values())
    facts = evidence.facts
    cells = judgement.cells
    lines = [
        f"# claw-sdk leak test: {evidence.image}",
        "",
        f"revision {facts.revision if facts else '?'} · seed `{evidence.seed}` · "
        f"exit {judgement.exit_code} ({_exit_words(judgement.exit_code)})",
        "",
    ]
    for name, value in judgement.headline.items():
        lines.append(f"- **{name}**: {value}")
    if facts:
        lines.append(f"- **FX-F in this revision (git)**: {facts.fx_f_in_revision}")
        lines.append(f"- **clipboard flags**: {facts.clipboard_flags}")
        oopif = ", ".join(
            f"{k}={'OOPIF' if v else 'in-process'}" for k, v in facts.oopif.items()
        )
        lines.append(f"- **frames observed**: {oopif}")
        for check, answer in facts.checks.items():
            lines.append(f"- preflight {check}: {answer}")
    for failure in judgement.failures:
        lines.append(f"- **harness failure** {failure}")
    counts = _counts(cells)
    lines += [
        "",
        "## Summary",
        "",
        ", ".join(f"{k}: {v}" for k, v in counts.items()),
        "",
        "## Table",
        "",
        "Rows are channels, columns are field kinds. A cell shows the worst verdict among "
        "its observations, tagged with where (location codes) and when (phase).",
        "",
    ]
    grid = table(judgement)
    present = judgement.present
    kinds = [k for k in Kind if k in present or any(k in row for row in grid.values())]
    lines.append("| channel | " + " | ".join(KINDS[k].column for k in kinds) + " |")
    lines.append("|---|" + "---|" * len(kinds))
    by_cell: dict[tuple[ChannelId, Kind], list[Cell]] = {}
    for cell in cells:
        by_cell.setdefault((cell.channel, cell.region.plant.kind), []).append(cell)
    for channel in ChannelId:
        only_searched = channel in SEARCH_ONLY and channel in judgement.searched
        if channel not in grid and not only_searched:
            continue
        row = []
        for kind in kinds:
            worst = grid.get(channel, {}).get(kind)
            if worst is None:
                row.append("clean" if only_searched and kind in present else "·")
                continue
            tags = (
                _tags(by_cell[(channel, kind)], worst) if cell_rank(worst) < 5 else ""
            )
            row.append(f"{glyph(worst)} {tags}".strip())
        lines.append(f"| {channel} | " + " | ".join(row) + " |")
    lines += [
        "",
        "legend: ok masked · held withheld by contract · LEAK new leak · ?? unproven · "
        "a residual id (fx-f, xsite, short, l2, r1, r3, upload) a known residual · "
        "raw the control's expected read · nb not built · clean searched, nothing found "
        "(a channel the judge can only search) · · not applicable or not exercised. "
        "In the keyboard chord rows, held means Neo refused the chord and ok means Neo "
        "sent it and the clipboard kept the baseline; LEAK there includes a value "
        "laundered into the paste box.",
        "",
        "## New leaks",
        "",
    ]
    new = [c for c in cells if cell_rank(c) == 0]
    lines += _leak_blocks(new, evidence, detector) or ["None."]
    lines += ["", "## Known residuals", ""]
    for residual in RESIDUALS:
        hits = [
            c
            for c in cells
            if c.residual == residual.id and c.verdict is not Verdict.MASKED
        ]
        if residual.status is Status.CONTROL and hits:
            lines.append(
                f"- **{residual.id}** ({residual.status}): {len(hits)} reads. {residual.why}"
            )
            continue
        if hits:
            channels = sorted({str(c.channel) for c in hits})
            lines.append(
                f"- **{residual.id}** ({residual.status}): {len(hits)} cells in "
                f"{', '.join(channels[:6])}{'…' if len(channels) > 6 else ''}. {residual.why}"
            )
        elif residual.status is Status.IN_FLIGHT:
            lines.append(
                f"- **{residual.id}** ({residual.status}): no longer observed. Drop it."
            )
    lines += [
        "",
        "## Searched",
        "",
        "| channel | answers | bytes | leaked cells |",
        "|---|---|---|---|",
    ]
    for channel in ChannelId:
        if channel in judgement.searched:
            answers, size = judgement.searched[channel]
            leaked = sum(
                1 for c in cells if c.channel is channel and c.verdict is Verdict.LEAKED
            )
            lines.append(f"| {channel} | {answers} | {size} | {leaked} |")
    lines += ["", "## Unproven cells", ""]
    unproven: dict[str, list[Cell]] = {}
    for cell in cells:
        if cell.verdict is Verdict.INCONCLUSIVE:
            unproven.setdefault(f"{cell.channel}: {cell.why}", []).append(cell)
    for reason, group in sorted(unproven.items()):
        where = sorted({PLACES[c.region.plant.location].code for c in group})
        lines.append(
            f"- {reason} ({len(group)} cells; {','.join(where)}; e.g. {group[0].source})"
        )
    if not unproven:
        lines.append("None.")
    return detector.redact("\n".join(lines) + "\n")


def _exit_words(code: int) -> str:
    words = [
        (EXIT_NEW_LEAK, "new leak"),
        (EXIT_KNOWN, "known residual"),
        (EXIT_INCONCLUSIVE, "unproven cells"),
        (EXIT_HARNESS, "harness failure"),
    ]
    return " + ".join(w for bit, w in words if code & bit) or "nothing left Neo"


def _counts(cells: Sequence[Cell]) -> dict[str, int]:
    counts = {
        "masked": 0,
        "new leaks": 0,
        "residual leaks": 0,
        "held": 0,
        "unproven": 0,
        "n/a": 0,
        "control raw": 0,
    }
    for cell in cells:
        rank = cell_rank(cell)
        key = {
            0: "new leaks",
            1: "unproven",
            2: "residual leaks",
            3: "residual leaks",
            4: "held",
            5: "masked",
            6: "held",
            7: "control raw",
            8: "n/a",
        }[rank]
        counts[key] += 1
    return counts


_EXCERPT = 2000  # characters of a request body the report shows inline


def _leak_blocks(
    cells: Sequence[Cell], evidence: Evidence, detector: Detector
) -> list[str]:
    """One block per leaking (channel, plant, phase) of an agent answer, with its exact
    request, a redacted context and a repro; record and disk copies of the same plant
    are listed under it. A plant only records or disk hold gets a block of its own, and
    when that record is a run's, the block also names the agent run that made it. Neo's
    own secrets copy no read: each channel that answered one gets a block of its own,
    with its first answer's request, and lists the rest of that channel's answers."""
    captures = {c.id: c for c in evidence.captures}
    streams = {s.id: s for s in evidence.streams}
    hunted = {secret.subject.id for secret in NEO_SECRETS}
    agent = [
        c
        for c in cells
        if SPECS[c.channel].family in AGENT_FAMILIES
        and c.channel is not ChannelId.REPLAY
        and c.region.id not in hunted
    ]
    runs = {
        c.run_id: c
        for c in evidence.captures
        if c.run_id
        and SPECS[c.channel].family in AGENT_FAMILIES
        and c.channel is not ChannelId.REPLAY
    }
    copies: dict[str, list[Cell]] = {}
    secrets: dict[tuple, list[Cell]] = {}
    for cell in cells:
        if cell.region.id in hunted:
            secrets.setdefault((cell.channel, cell.region.id), []).append(cell)
        elif cell not in agent:
            copies.setdefault(cell.region.plant.id, []).append(cell)
    leads: dict[tuple, Cell] = {}
    for cell in sorted(agent, key=lambda c: (str(c.channel), c.region.id)):
        leads.setdefault((cell.channel, cell.region.plant.id, cell.phase), cell)
    for plant_id, rest in copies.items():
        if not any(key[1] == plant_id for key in leads):
            leads[(rest[0].channel, plant_id, rest[0].phase)] = rest[0]
    leads.update({key: group[0] for key, group in secrets.items()})
    out: list[str] = []
    for n, cell in enumerate(leads.values(), 1):
        plant = cell.region.plant
        source = captures.get(cell.source) or streams.get(cell.source)
        out.append(
            f"{n}. **{cell.channel}** · {KINDS[plant.kind].column} "
            f"{placeholder(plant, cell.region)} · {cell.region.exposure} in "
            f"{PLACES[cell.region.location].code} · {cell.phase} · {cell.why}"
        )
        if source is not None:
            out += _request_lines("request", source, evidence, detector)
        else:
            out.append(f"   - evidence: `{cell.source}`")
        run = None if cell in agent else runs.get(cell.run_id or "")
        if run is not None and run is not source:
            out += _request_lines("the agent run it records", run, evidence, detector)
        if cell.hit:
            out.append(f"   - context: `{cell.hit.context}`")
        same = [c for c in copies.get(plant.id, []) if c is not cell]
        if same and cell in agent:
            where = sorted({f"{c.channel} ({c.source})" for c in same})
            out.append(f"   - also in {len(where)} records: {'; '.join(where[:8])}")
        more = sorted(
            {c.source for c in secrets.get((cell.channel, cell.region.id), ())}
            - {cell.source}
        )
        if more:
            noun = "answer" if len(more) == 1 else "answers"
            out.append(f"   - also in {len(more)} more {noun}: {'; '.join(more[:8])}")
    return out


def _request_lines(
    label: str, source: Capture | Stream, evidence: Evidence, detector: Detector
) -> list[str]:
    """A request as a leak block prints it: redacted, cut to _EXCERPT, with the line of
    log.jsonl that holds it whole, its evidence and a repro."""
    req = source.request
    body = detector.redact(req.body or "")
    whole = evidence.lines.get(source.id)
    where = f"; whole in `log.jsonl` line {whole}" if whole else ""
    cut = (
        f", cut at {_EXCERPT} of {len(body)} characters" if len(body) > _EXCERPT else ""
    )
    lines = [f"   - {label}: `{req.transport} {req.method} {req.target}`{cut}{where}"]
    if body:
        lines.append("     ```\n" + _indent(body[:_EXCERPT], "     ") + "\n     ```")
    lines.append(
        f"   - evidence: `{source.blob}`; repro: `python3 leak_sdk.py --image "
        f"{evidence.image} --scenario {source.scenario} --keep` (this run: "
        f"instance {source.instance}, seed {evidence.seed})"
    )
    return lines


def _indent(text: str, prefix: str) -> str:
    return "\n".join(prefix + line for line in text.splitlines())


def render_json(judgement: Judgement, evidence: Evidence) -> str:
    """verdicts.json: every cell, for machines and for diffing two runs. Every string in
    it passes the run's redactor, as report.md does; strings repeat, so each is
    redacted once."""
    data = {
        "exit": judgement.exit_code,
        "headline": dict(judgement.headline),
        "failures": list(judgement.failures),
        "cells": [
            {
                "id": c.id,
                "channel": str(c.channel),
                "kind": str(c.region.plant.kind),
                "plant": c.region.plant.id,
                "region": c.region.id,
                "exposure": str(c.region.exposure),
                "location": str(c.region.location),
                "plant_location": str(c.region.plant.location),
                "phase": str(c.phase),
                "verdict": str(c.verdict),
                "why": c.why,
                "residual": c.residual,
                "source": c.source,
                "run": c.run_id,
                "derived_from": c.derived_from,
                "form": str(c.hit.form) if c.hit else None,
            }
            for c in judgement.cells
        ],
    }
    redact = functools.cache(Detector(evidence.instances.values()).redact)
    return json.dumps(_redacted(data, redact), indent=1)


def _redacted(node: object, redact: Callable[[str], str]) -> object:
    if isinstance(node, str):
        return redact(node)
    if isinstance(node, dict):
        return {key: _redacted(value, redact) for key, value in node.items()}
    if isinstance(node, list):
        return [_redacted(value, redact) for value in node]
    return node
