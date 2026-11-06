"""What the claw-sdk leak test plants and how it probes: the plan half of the test.

leak_sdk.py drives a real Neo container and records every byte; leak_judge.py turns that
record into verdicts. This module owns what both need and neither may copy:

- the planted values: sensitive kinds x frame locations x the ways a page exposes them,
  fresh for every page instance (`make_instance`), and the per-document slice the fixture
  renders (`Instance.slice`);
- every channel's probe: the run code that reads it, what it reaches by contract, and
  which answers are the contract refusing (`SPECS`, one entry per `ChannelId`);
- the two rules that keep results honest: a cold probe is the first touch of values
  nothing has seen (`PhaseGuard`), and no planted value or coverage anchor ever rides in a
  request the harness sends, outside the one way in each origin declares (`Secret`,
  `assert_clean`, `DECLARED_WAYS_IN`).

Pure: no I/O, no clock, and no randomness beyond the run's seed.
"""

from __future__ import annotations

import dataclasses
import json
import posixpath
import random
import re
from collections.abc import Callable, Collection, Iterable, Mapping, Sequence
from enum import StrEnum

FIXTURE_PORT = 8800


class Location(StrEnum):
    """Where a planted field or a page echo sits in one fixture instance.

    The frame tree of an instance named i07-owner:

        http://a.test:8800/fx/i07-owner/main         MAIN, SHADOW_OPEN, SHADOW_CLOSED
        |- #fr-so   a.test/…/same_origin             SAME_ORIGIN
        |- #fr-ss   sub.a.test/…/same_site           SAME_SITE (cross-origin, same site)
        |- #fr-xo   b.test/…/cross_site              CROSS_SITE
        |  `- #fr-xn a.test/…/nested                 NESTED (a.test inside b.test)
        |- #fr-nav  b.test/…/nav_touched             NAV_TOUCHED (OWNER layout)
        `- #fr-late b.test/…/nav_late                NAV_LATE (added during control)
        popup tab   a.test/…/popup                   POPUP (opened by the page)
    """

    MAIN = "main"
    SAME_ORIGIN = "same_origin"
    SAME_SITE = "same_site"
    CROSS_SITE = "cross_site"
    NESTED = "nested"
    SHADOW_OPEN = "shadow_open"
    SHADOW_CLOSED = "shadow_closed"
    NAV_TOUCHED = "nav_touched"
    NAV_LATE = "nav_late"
    POPUP = "popup"


@dataclasses.dataclass(frozen=True)
class Place:
    """The facts about one location that the plan, the fixture and the judge all read."""

    code: str  # short label in table cells
    host: str  # an --add-host name; the document loads from http://<host>:8800
    document: Location  # the document that holds it: MAIN for the two shadow roots
    frame_path: tuple[str, ...]  # iframe element ids from the top, for frameLocator
    expect_oopif: bool  # what preflight should see in Target.getTargets
    same_site_as_top: bool  # inside a site-wide sweep of the top frame's site
    agent_reachable: bool  # False only where no selector reaches: the closed root
    other_tab: bool = False  # lives in a tab of its own


PLACES: Mapping[Location, Place] = {
    Location.MAIN: Place("M", "a.test", Location.MAIN, (), False, True, True),
    Location.SAME_ORIGIN: Place(
        "SO", "a.test", Location.SAME_ORIGIN, ("fr-so",), False, True, True
    ),
    Location.SAME_SITE: Place(
        "SS", "sub.a.test", Location.SAME_SITE, ("fr-ss",), False, True, True
    ),
    Location.CROSS_SITE: Place(
        "XO", "b.test", Location.CROSS_SITE, ("fr-xo",), True, False, True
    ),
    Location.NESTED: Place(
        "XN", "a.test", Location.NESTED, ("fr-xo", "fr-xn"), True, True, True
    ),
    Location.SHADOW_OPEN: Place("SH", "a.test", Location.MAIN, (), False, True, True),
    Location.SHADOW_CLOSED: Place(
        "SC", "a.test", Location.MAIN, (), False, True, False
    ),
    Location.NAV_TOUCHED: Place(
        "NT", "b.test", Location.NAV_TOUCHED, ("fr-nav",), True, False, True
    ),
    Location.NAV_LATE: Place(
        "NL", "b.test", Location.NAV_LATE, ("fr-late",), True, False, True
    ),
    Location.POPUP: Place("PU", "a.test", Location.POPUP, (), False, True, True, True),
}

# The brief's frame matrix: every sensitive kind is planted in each of these.
FRAME_MATRIX: tuple[Location, ...] = (
    Location.MAIN,
    Location.SAME_ORIGIN,
    Location.SAME_SITE,
    Location.CROSS_SITE,
    Location.NESTED,
    Location.SHADOW_OPEN,
    Location.SHADOW_CLOSED,
)
AGENT_FIELDS = tuple(loc for loc in FRAME_MATRIX if PLACES[loc].agent_reachable)
NAV_FRAMES = (Location.NAV_TOUCHED, Location.NAV_LATE)


def parent_document(document: Location) -> Location | None:
    """The document whose iframe holds `document`; None for top-level documents."""
    path = PLACES[document].frame_path
    if not path:
        return None
    for loc, place in PLACES.items():
        if place.document is loc and place.frame_path == path[:-1]:
            return loc
    raise AssertionError(f"no parent document for {document}")


class Kind(StrEnum):
    """A row of the D2 rule table as one planted field: the report's columns, in order."""

    PASSWORD = "password"
    CURRENT_PASSWORD = "current_password"
    NEW_PASSWORD = "new_password"
    OTP = "otp"
    OTP_SPLIT = "otp_split"
    CC_NUMBER = "cc_number"
    CC_CSC = "cc_csc"
    WORD_CARD = "word_card"
    WORD_PASSWORD = "word_password"
    WORD_OTP = "word_otp"
    SHOW_PASSWORD = "show_password"
    GAP_EARLY_TOGGLE = "gap_early_toggle"  # no rule on purpose: design risk R3
    GAP_UNLABELLED = "gap_unlabelled"  # no rule on purpose: design risk R1
    PLAIN = "plain"  # the control: not sensitive, reads back raw
    FILE = "file"  # a file input: it holds what the page read from the uploaded file


class Shape(StrEnum):
    """How a kind's value is generated, which decides how it can be detected."""

    TEXT = "text"  # 12 letters and digits, no confusables: free-text detectable
    SPECIAL = "special"  # TEXT plus & < " (one per instance): every escaping form
    CARD = "card"  # 16-digit Luhn: detected whole or grouped, never as fragments
    CVC = "cvc"  # 3 digits: short, anchored slots only
    DIGITS = "digits"  # a split code, one digit per box: short, anchored slots only


@dataclasses.dataclass(frozen=True)
class KindSpec:
    """How the fixture renders one kind and which rule is meant to classify it.

    `attrs` and `words` are the whole field as the page renders it: the slice carries
    them and fx.js renders them verbatim, so `lint_instance` checks exactly what the
    classifier sees. `words` precede the region's anchor in the field's <label>.
    """

    column: str  # the table header
    rule: str  # the D2 rule meant to classify it, for the report's legend
    sensitive: bool  # False for PLAIN and the two gaps: raw is the expected answer
    shape: Shape
    attrs: tuple[tuple[str, str], ...]
    words: str = "Field"


_TEXT = (("type", "text"),)
_PASSWORD = (("type", "password"),)
KINDS: Mapping[Kind, KindSpec] = {
    Kind.PASSWORD: KindSpec("pw", "type=password", True, Shape.TEXT, _PASSWORD),
    Kind.CURRENT_PASSWORD: KindSpec(
        "cur",
        "autocomplete current-password (type=text)",
        True,
        Shape.TEXT,
        (*_TEXT, ("autocomplete", "current-password")),
    ),
    Kind.NEW_PASSWORD: KindSpec(
        "new",
        "autocomplete new-password (type=text)",
        True,
        Shape.TEXT,
        (*_TEXT, ("autocomplete", "new-password")),
    ),
    Kind.OTP: KindSpec(
        "otp",
        "autocomplete one-time-code",
        True,
        Shape.TEXT,
        (*_TEXT, ("autocomplete", "one-time-code")),
    ),
    Kind.OTP_SPLIT: KindSpec(
        "split",
        "maxlength=1 numeric boxes in a form that says verification code",
        True,
        Shape.DIGITS,
        (*_TEXT, ("maxlength", "1"), ("inputmode", "numeric")),
        "Box",
    ),
    Kind.CC_NUMBER: KindSpec(
        "cc#",
        "autocomplete cc-number",
        True,
        Shape.CARD,
        (*_TEXT, ("autocomplete", "cc-number"), ("inputmode", "numeric")),
    ),
    Kind.CC_CSC: KindSpec(
        "cvc",
        "autocomplete cc-csc",
        True,
        Shape.CVC,
        (*_TEXT, ("autocomplete", "cc-csc"), ("inputmode", "numeric")),
    ),
    Kind.WORD_CARD: KindSpec(
        "w:card", "label words 'Card number'", True, Shape.CARD, _TEXT, "Card number"
    ),
    Kind.WORD_PASSWORD: KindSpec(
        "w:pw",
        "placeholder word 'Passcode'",
        True,
        Shape.TEXT,
        (*_TEXT, ("placeholder", "Passcode")),
    ),
    Kind.WORD_OTP: KindSpec(
        "w:otp", "name=otp", True, Shape.TEXT, (*_TEXT, ("name", "otp"))
    ),
    Kind.SHOW_PASSWORD: KindSpec(
        "show",
        "was type=password when the seal watched it",
        True,
        Shape.TEXT,
        _PASSWORD,
    ),
    Kind.GAP_EARLY_TOGGLE: KindSpec(
        "gap:R3",
        "none: flipped to text before Neo's first touch",
        False,
        Shape.TEXT,
        _PASSWORD,
    ),
    Kind.GAP_UNLABELLED: KindSpec(
        "gap:R1", "none: a card number in a plain box", False, Shape.CARD, _TEXT
    ),
    Kind.PLAIN: KindSpec("plain", "none: the control", False, Shape.TEXT, _TEXT),
    Kind.FILE: KindSpec(
        "file",
        "none: no rule sees a file's content",
        False,
        Shape.TEXT,
        (("type", "file"),),
    ),
}
SENSITIVE_KINDS = tuple(k for k, s in KINDS.items() if s.sensitive)
# Every kind a page's own form field can be: an upload's file input only in UPLOAD.
FIELD_KINDS = tuple(k for k in Kind if k is not Kind.FILE)
GAP_KINDS = (Kind.GAP_EARLY_TOGGLE, Kind.GAP_UNLABELLED)
# The split code's form text: what makes its boxes one-time-code fields (codeSlot).
SPLIT_FORM_TEXT = "Enter the verification code"
SPLIT_BOXES = 6


class Exposure(StrEnum):
    """Where in the page a region shows its plant's value."""

    FIELD = "field"  # in the field itself
    ECHO = "echo"  # copied by the page into the top document's text and a data-echo
    TITLE = "title"  # copied into document.title
    URL = "url"  # copied into the URL by history.pushState
    DIALOG = "dialog"  # copied into an alert()
    DOWNLOAD = "download"  # copied into a download's file name
    POPUP_TITLE = "popup_title"  # copied into a popup tab's title
    SERVER_ECHO = (
        "server_echo"  # submitted; the next document shows it, the field is gone
    )
    PASTED = "pasted"  # the owner copied it and pasted it into a plain box


# Exposures whose text the page writes as "ECHO <anchor> {{value}}": the {{ }} delimiters
# survive JSON and HTML escaping, so the judge reads a short value's slot exactly.
ECHO_LIKE = frozenset(set(Exposure) - {Exposure.FIELD, Exposure.PASTED})


class Origin(StrEnum):
    """Who put the value into the page. Only AGENT values came to Neo from the agent, each
    in the code of the one run that typed it; every other exit must still hide them."""

    PAGE = "page"  # the page's script set it (prefill, the page's own input events)
    OWNER = "owner"  # typed on the control display
    HOST = "host"  # known only to the host, which registers it with POST /v1/secrets
    AGENT = "agent"  # in one run's code: typed by keyboard.type, or an upload's buffer
    FILE = (
        "file"  # a file on Neo's disk, which the agent's setInputFiles hands the page
    )
    # A file in Neo's downloads dir, handed over the same way: agents re-upload what they
    # downloaded, so the deny list must leave that dir alone (decision 33b).
    DOWNLOAD = "download"
    # One of Neo's own secret files, which setInputFiles must refuse (decision 33b). Its
    # bytes never leave the container: the oracle compares what the page read with the
    # file there (deny_uploads). The plant's value is a placeholder no page ever holds,
    # kept so the plant has the shape every other plant has.
    NEO = "neo"


# The declared ways a planted value may ride a request the harness sends, by origin: the
# one request that hands it to Neo by design. assert_clean refuses a value anywhere else.
# OWNER keys travel as RFB key events on the control display, which assert_clean never
# reads (they are not text the harness formats), so they need no entry here; nor does a
# FILE or DOWNLOAD value, which the oracle writes onto Neo's disk and no request carries.
DECLARED_WAYS_IN: Mapping[Origin, str] = {
    Origin.HOST: "http POST /v1/secrets",
    Origin.AGENT: "mcp tools/call run",
}


class Secret:
    """One planted value. `str()`, `format()` and f-strings raise; only `reveal()` gives it.

    reveal() has eight callers, each a designed way in or a reader of the record: the
    slices the oracle writes into the container, the file it writes on Neo's disk, the
    owner's keystrokes, POST /v1/secrets, the agent's typing run (`typing_code`) and
    upload buffer (`buffer_code`), the evidence manifest, and the judge's needles. Everything else handles plants by id, and `assert_clean` backs this
    up at the wire.
    """

    __slots__ = ("_value",)

    def __init__(self, value: str) -> None:
        self._value = value

    def reveal(self) -> str:
        return self._value

    def __len__(self) -> int:
        return len(self._value)

    def __str__(self) -> str:
        raise TypeError("a planted value is never formatted; use its plant id")

    def __format__(self, spec: str) -> str:
        raise TypeError("a planted value is never formatted; use its plant id")

    def __repr__(self) -> str:
        return f"Secret(<{len(self._value)} chars>)"


@dataclasses.dataclass(frozen=True)
class Plant:
    """A value Neo did not get from the agent, first entered into one field.

    Values are unique across the whole run, so a long value found anywhere names one
    plant. A split code is one plant whose boxes are regions with `part` set.
    """

    id: str  # "i07.p03"
    kind: Kind
    location: Location  # where its field is (where it is shown, for a HOST plant)
    origin: Origin
    secret: Secret
    late: bool = False  # PAGE-origin, set with input events only on Act.LATE_PLANT
    navigates: bool = False  # the owner submits it; the next document echoes it

    @property
    def registrable(self) -> bool:
        """What a sweep or the journal adds to Neo's SecretSet: a sensitive value of 4+
        characters in one field. Shorter values, and a split code (one digit per box),
        are L1's alone (and BD-SHORT as echoes)."""
        return (
            KINDS[self.kind].sensitive
            and self.kind is not Kind.OTP_SPLIT
            and len(self.secret) >= 4
        )


@dataclasses.dataclass(frozen=True)
class Region:
    """One place an instance shows a plant's value: the unit every verdict is about.

    `id` is the DOM id and the selector the run code uses; it is unique across the run.
    `anchor` is an opaque token the page renders next to the value (a field's label, an
    echo's prefix) and the harness never sends: finding it in a channel's bytes proves the
    channel read this region, which an id echoed back from the run's own code could not.
    """

    id: str  # "r07012"
    anchor: str  # "zqk4tbm7"
    plant: Plant
    exposure: Exposure
    location: Location  # where it sits (an echo of a CROSS_SITE plant sits in MAIN)
    part: int | None = None  # a split-code box: which digit of the plant it holds

    def expected(self) -> str:
        """The raw text this region holds: what a leak looks like in its slot."""
        value = self.plant.secret.reveal()
        return value if self.part is None else value[self.part]

    @property
    def short(self) -> bool:
        """Found only in anchored slots: a CVC, a split box, a joined split code."""
        return self.plant.kind is Kind.OTP_SPLIT or len(self.expected()) < 4


class Layout(StrEnum):
    """What an instance plants. One fixture renders every layout from its slices."""

    FIELDS = "fields"  # every kind x the frame matrix, prefilled; no echoes
    ECHOES = "echoes"  # fields + echoes in the top document of every value
    FACTS = (
        "facts"  # a few fields; acts copy them into title, URL, alert, download, popup
    )
    PIXELS = "pixels"  # a few large fields at the top of the viewport
    OWNER = "owner"  # empty fields the owner types into, live echoes, nav frames
    HOST = "host"  # a value only the host knows, shown in text with no field
    COPY = "copy"  # FIELDS, plus a plain baseline field and a paste box in MAIN
    LOCKED = "locked"  # FIELDS, every field read-only: an action waits on it in vain
    TYPED = "typed"  # empty fields the agent types into, with live echoes in MAIN
    SCRIPT = "script"  # ECHOES, plus a popup document for reads through window.opener
    # Same-site frames whose fields plant themselves, echoed and titled in a top document
    # the agent writes with page.setContent (content_html): the fixture serves it a shell.
    CONTENT = "content"
    # File inputs for two files on Neo's disk (one in its downloads dir) and one for the
    # agent's own buffer, each echoed in MAIN; one per deny-list probe, never echoed; and
    # the fixture's own input for the read-back's control (DIGEST_CONTROL).
    UPLOAD = "upload"


# Documents a layout serves beyond those its regions sit in: the popup a SCRIPT page opens
# holds nothing of its own; a main-world read in it reaches the page through window.opener.
EXTRA_DOCUMENTS: Mapping[Layout, tuple[Location, ...]] = {
    Layout.SCRIPT: (Location.POPUP,),
}


@dataclasses.dataclass(frozen=True)
class Instance:
    """One fixture page: one tab, one fresh value set. The unit of freshness."""

    name: str  # "i07-echoes": taskId, URL path segment, requestId prefix
    layout: Layout
    plants: tuple[Plant, ...]
    regions: tuple[Region, ...]

    def url(self, location: Location = Location.MAIN) -> str:
        """The address of the document holding `location`. Holds no value, no anchor."""
        place = PLACES[location]
        return f"http://{place.host}:{FIXTURE_PORT}/fx/{self.name}/{place.document}"

    def region(self, region_id: str) -> Region:
        return next(r for r in self.regions if r.id == region_id)

    def regions_of(self, plant: Plant) -> tuple[Region, ...]:
        return tuple(r for r in self.regions if r.plant is plant)

    def field_regions(self, plant: Plant) -> tuple[Region, ...]:
        """The plant's field boxes in part order (one box unless it is a split code)."""
        return tuple(r for r in self.regions_of(plant) if r.exposure is Exposure.FIELD)

    def plant(
        self,
        kind: Kind | None = None,
        location: Location | None = None,
        *,
        origin: Origin | None = None,
    ) -> Plant:
        """The one plant matching every given filter; raises unless exactly one does."""
        found = [
            p
            for p in self.plants
            if (kind is None or p.kind is kind)
            and (location is None or p.location is location)
            and (origin is None or p.origin is origin)
        ]
        if len(found) != 1:
            raise LookupError(f"{self.name}: {len(found)} plants match")
        return found[0]

    def owner_plants(
        self, *, navigating: bool = False, kinds: Collection[Kind] | None = None
    ) -> tuple[Plant, ...]:
        """Plants the owner types, in typing order (frame matrix order, then kind)."""
        return tuple(
            p
            for p in self.plants
            if p.origin is Origin.OWNER
            and p.navigates is navigating
            and (kinds is None or p.kind in kinds)
        )

    def documents(self) -> tuple[Location, ...]:
        """Every document of the instance, parents before children (popup included)."""
        docs = {PLACES[r.location].document for r in self.regions}
        docs |= set(EXTRA_DOCUMENTS.get(self.layout, ()))
        for doc in list(docs):
            while (parent := parent_document(doc)) is not None:
                docs.add(parent)
                doc = parent
        docs.add(Location.MAIN)
        return tuple(loc for loc in Location if loc in docs)

    def slice(self, document: Location) -> dict:
        """The JSON the fixture document of `document` fetches (no-store) and renders.

        Its regions (fields with the attributes and label words to render, echoes with
        the field regions that feed them; values only for what the page itself plants),
        its child frames, and in MAIN the two shadow roots' regions (marked by
        `location`). Server echoes are not here: the fixture server renders them after
        a submit (`server_echoes`).
        """
        regions = [
            self._region_json(r)
            for r in self.regions
            if PLACES[r.location].document is document
            and r.exposure is not Exposure.SERVER_ECHO
        ]
        frames = [
            {
                "id": PLACES[child].frame_path[-1],
                "src": self.url(child),
                "late": child is Location.NAV_LATE,
            }
            for child in self.documents()
            if child is not document
            and not PLACES[child].other_tab
            and parent_document(child) is document
        ]
        return {
            "instance": self.name,
            "layout": str(self.layout),
            "location": str(document),
            "regions": regions,
            "frames": frames,
            # The agent writes this document with setContent: the fixture serves a shell
            # until then, and the fixture script the agent's HTML loads adds no frames.
            "written": self.layout is Layout.CONTENT and document is Location.MAIN,
        }

    def server_echoes(self) -> dict[str, dict]:
        """Field region id -> the SERVER_ECHO region the next document renders after the
        owner submits that field. The fixture server builds that document's slice from
        the posted value, so the value never sits in a URL or in anything the harness
        sends."""
        out = {}
        for r in self.regions:
            if r.exposure is Exposure.SERVER_ECHO:
                (field,) = self.field_regions(r.plant)
                out[field.id] = {
                    "instance": self.name,
                    "layout": str(self.layout),
                    "location": str(r.location),
                    "region": self._region_json(r),
                }
        return out

    def _region_json(self, r: Region) -> dict:
        plant = r.plant
        out: dict = {
            "id": r.id,
            "anchor": r.anchor,
            "kind": str(plant.kind),
            "exposure": str(r.exposure),
            "location": str(r.location),
            "plant": plant.id,
            "late": plant.late,
            "navigates": plant.navigates,
        }
        if r.part is not None:
            out["part"] = r.part
        if r.exposure is Exposure.FIELD:
            spec = KINDS[plant.kind]
            out["attrs"] = [list(a) for a in spec.attrs]
            out["words"] = spec.words
            if plant.origin is Origin.PAGE:
                out["value"] = r.expected()
            if plant.origin is Origin.NEO:
                # A deny-list probe's input: the page hashes what it reads there for the
                # oracle and never echoes, mirrors or proves it (fx.js watchDigests).
                out["digests"] = True
                (probe,) = [u.probe for u in deny_uploads(self) if u.region.id == r.id]
                if probe.directory:
                    out["attrs"].append(["webkitdirectory", ""])
        elif r.exposure is Exposure.PASTED:
            out["attrs"] = [list(a) for a in _TEXT]
            out["words"] = "Field"
        elif plant.origin is Origin.HOST:
            out["value"] = r.expected()
        else:
            out["sources"] = [f.id for f in self.field_regions(plant)]
        return out


# TEXT values drop characters OCR confuses (I l 1 O o 0 c) and start with a letter, so
# none can match a run of digits (timestamps, ids, JPEG bytes). Ids and anchors also
# drop vowels, c, p and v, so no neutral token can spell a rule word (otp, pin, cvc).
_TEXT_ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZabdefghjkmnqrstuwxyz23456789"
_LETTERS = "".join(ch for ch in _TEXT_ALPHABET if ch.isalpha())
_ANCHOR_ALPHABET = "bdfghjkmnqrstwxz2345678"
VALUE_LENGTH = 12  # >= 8 alphanumerics in a row: every escaping form keeps the core
FRAGMENT = 8  # this many consecutive characters of a TEXT value are a (partial) leak
ANCHOR_LENGTH = 8
SPECIAL_TAIL = '&<"'  # the SPECIAL value's escaping probe, after its alphanumeric core


def luhn_ok(digits: str) -> bool:
    total = 0
    for i, ch in enumerate(reversed(digits)):
        d = int(ch)
        if i % 2:
            d = d * 2 - 9 if d > 4 else d * 2
        total += d
    return total % 10 == 0


def _value(shape: Shape, rng: random.Random) -> str:
    if shape in (Shape.TEXT, Shape.SPECIAL):
        core = rng.choice(_LETTERS) + "".join(
            rng.choice(_TEXT_ALPHABET) for _ in range(VALUE_LENGTH - 1)
        )
        return core + SPECIAL_TAIL if shape is Shape.SPECIAL else core
    if shape is Shape.CARD:
        body = "4" + "".join(rng.choice("0123456789") for _ in range(14))
        return next(body + c for c in "0123456789" if luhn_ok(body + c))
    if shape is Shape.CVC:
        return "".join(rng.choice("123456789") for _ in range(3))
    return "".join(rng.choice("0123456789") for _ in range(SPLIT_BOXES))


def _token(rng: random.Random) -> str:
    return "".join(rng.choice(_ANCHOR_ALPHABET) for _ in range(ANCHOR_LENGTH))


# (kind, location, origin, navigates) for every plant of a layout, and which exposures
# (beyond the field) each plant gets.
PlantSpec = tuple[Kind, Location, Origin, bool]


def _layout_plants(layout: Layout) -> list[PlantSpec]:
    page, owner = Origin.PAGE, Origin.OWNER
    if layout in (Layout.FIELDS, Layout.COPY, Layout.LOCKED):
        return [
            (kind, loc, page, False)
            for loc in FRAME_MATRIX
            for kind in (*SENSITIVE_KINDS, Kind.PLAIN)
        ] + [(kind, Location.MAIN, page, False) for kind in GAP_KINDS]
    if layout is Layout.TYPED:
        # Typing order. The closed root goes first: the seal cannot hear input there, so
        # typing that comes last is a journaled field's, and a read right after it still
        # meets every value held, before the journal's quiet registers them. PLAIN is the
        # control: no op on it is sensitive, so its run's code is stored as sent.
        order = (Location.SHADOW_CLOSED, *FRAME_MATRIX[:-1])
        return [(Kind.PLAIN, Location.MAIN, Origin.AGENT, False)] + [
            (kind, loc, Origin.AGENT, False)
            for loc in order
            for kind in SENSITIVE_KINDS
        ]
    if layout in (Layout.ECHOES, Layout.SCRIPT):
        return [(kind, Location.MAIN, page, False) for kind in FIELD_KINDS] + [
            (kind, loc, page, False)
            for loc in FRAME_MATRIX[1:]
            for kind in (Kind.PASSWORD, Kind.CC_CSC)
        ]
    if layout is Layout.FACTS:
        return [
            (Kind.PASSWORD, Location.MAIN, page, False),
            (Kind.OTP, Location.MAIN, page, False),
            (Kind.PASSWORD, Location.CROSS_SITE, page, False),
        ]
    if layout is Layout.PIXELS:
        kinds = (Kind.CC_NUMBER, Kind.OTP, Kind.PASSWORD, Kind.SHOW_PASSWORD)
        return [(kind, Location.MAIN, page, False) for kind in kinds]
    if layout is Layout.UPLOAD:
        return [
            (Kind.FILE, Location.MAIN, Origin.FILE, False),
            (Kind.FILE, Location.MAIN, Origin.DOWNLOAD, False),
            (Kind.FILE, Location.MAIN, Origin.AGENT, False),
        ] + [(Kind.FILE, Location.MAIN, Origin.NEO, False)] * len(DENY_PROBES)
    if layout is Layout.CONTENT:
        # Frames of the page's site only: setContent runs between sweeps of that site, and
        # a cross-site frame's echo is XSITE whatever the op (measured in "echoes").
        frames = (Location.SAME_ORIGIN, Location.SAME_SITE, Location.NESTED)
        return [
            (kind, loc, page, False)
            for loc in frames
            for kind in (Kind.PASSWORD, Kind.CC_CSC)
        ]
    if layout is Layout.OWNER:
        return (
            [
                (kind, loc, owner, False)
                for loc in FRAME_MATRIX
                for kind in SENSITIVE_KINDS
            ]
            + [(Kind.PLAIN, loc, page, False) for loc in FRAME_MATRIX]
            + [
                (Kind.PASSWORD, loc, owner, True)
                for loc in (*NAV_FRAMES, Location.POPUP)
            ]
        )
    return [(Kind.PASSWORD, Location.MAIN, Origin.HOST, False)]


def _layout_exposures(layout: Layout, plant: Plant) -> list[tuple[Exposure, Location]]:
    """The non-field regions of `plant` in `layout`, with where each sits."""
    main = Location.MAIN
    if layout in (Layout.ECHOES, Layout.SCRIPT) and plant.kind is not Kind.PLAIN:
        return [(Exposure.ECHO, main)]
    if layout is Layout.FACTS:
        return [
            (Exposure.TITLE, main),
            (Exposure.URL, main),
            (Exposure.DIALOG, main),
            (Exposure.DOWNLOAD, main),
            (Exposure.POPUP_TITLE, Location.POPUP),
        ]
    if layout is Layout.PIXELS and plant.kind is Kind.OTP:
        return [(Exposure.URL, main)]
    if layout is Layout.CONTENT:
        return [(Exposure.ECHO, main), (Exposure.TITLE, main)]
    if layout is Layout.UPLOAD and plant.origin is not Origin.NEO:
        return [(Exposure.ECHO, main)]
    if layout is Layout.OWNER and plant.origin is Origin.OWNER:
        out = [(Exposure.ECHO, main)]
        if plant.navigates:
            out.append((Exposure.SERVER_ECHO, plant.location))
        if plant.kind is Kind.CC_NUMBER and plant.location is main:
            out.append((Exposure.PASTED, main))
        return out
    if layout in (Layout.HOST, Layout.TYPED):
        return [(Exposure.ECHO, main)]
    return []


def make_instance(
    layout: Layout, *, seed: str, index: int, late: bool = False
) -> Instance:
    """A fresh page instance, deterministic in (seed, index).

    Values carry no instance prefix (a shared prefix would be a core matching across
    plants); 12 draws from 51 symbols make a shared 8-gram among a run's ~2,000 values
    about 1e-6, and the judge's detector refuses to start if one happens. `late` leaves
    every PAGE field empty until Act.LATE_PLANT sets it with input events.
    """
    rng = random.Random(f"{seed}:{index}")
    name = f"i{index:02}-{layout}"
    counter = iter(range(1000))
    plants: list[Plant] = []
    regions: list[Region] = []
    special_given = False

    def region(plant: Plant, exposure: Exposure, loc: Location, part=None) -> None:
        regions.append(
            Region(
                f"r{index:02}{next(counter):03}",
                _token(rng),
                plant,
                exposure,
                loc,
                part,
            )
        )

    for n, (kind, loc, origin, navigates) in enumerate(_layout_plants(layout)):
        shape = KINDS[kind].shape
        if kind is Kind.PASSWORD and loc is Location.MAIN and not special_given:
            shape, special_given = Shape.SPECIAL, True
        plant = Plant(
            f"i{index:02}.p{n:02}",
            kind,
            loc,
            origin,
            Secret(_value(shape, rng)),
            late=late and origin is Origin.PAGE,
            navigates=navigates,
        )
        plants.append(plant)
        if origin is not Origin.HOST:
            boxes = SPLIT_BOXES if kind is Kind.OTP_SPLIT else 1
            for part in range(boxes):
                region(plant, Exposure.FIELD, loc, part if boxes > 1 else None)
    for plant in plants:
        for exposure, loc in _layout_exposures(layout, plant):
            region(plant, exposure, loc)
    return Instance(name, layout, tuple(plants), tuple(regions))


REGISTRABLE_BUDGET = 2048  # the SecretSet holds 4096; stay far from eviction


def registrable(plant: Plant, *, typed: bool) -> int:
    """How many SecretSet entries this plant can add once planted (typed=False) or typed.

    A registrable value adds one entry when swept. A typed one (by the owner or the
    agent) is held until its field is quiet and then registered whole, never its
    prefixes; a read that sweeps the field between two keystrokes holds the prefix it
    found beside it, so a typed value can add two (claw-sdk builtins/typed_values.rs, the
    rule the takeover's control journal follows too). The Session sums this as it plants
    and types and stops before REGISTRABLE_BUDGET, so the set's cap can never evict a
    value and fake a leak.
    """
    if not plant.registrable:
        return 0
    return 2 if typed else 1


# The COPY layout's two plain fields in MAIN, outside every plant: the baseline the agent
# copies first (a fresh token the page makes for each probe, never a planted value) and the
# box it pastes into. The oracle reports both digests under these ids (fx.js ownState).
COPY_BASE = "fx-base"
COPY_DROP = "fx-drop"


def neutral_tokens(instance: Instance) -> list[str]:
    """Every token the page renders that is meant to classify as nothing."""
    tokens = [instance.name, "ECHO", "Field", "Box", COPY_BASE, COPY_DROP]
    tokens.append(DIGEST_CONTROL)
    tokens += [r.id for r in instance.regions] + [r.anchor for r in instance.regions]
    tokens += [p.frame_path[-1] for p in PLACES.values() if p.frame_path]
    tokens += [str(loc) for loc in Location]
    return tokens


def lint_instance(instance: Instance, rules: Mapping) -> list[str]:
    """Every neutral token or field label that would classify by the wrong rule.

    `rules` is rules.json. A neutral token matching a word rule would classify a field
    by a rule it is not meant to test, so a broken rule could hide behind it; a word
    kind must classify as its own kind by its words alone; the split code's form text
    must meet codeSlot.context.
    """
    words = [(w["kind"], re.compile(w["pattern"], re.I)) for w in rules["words"]]

    def word_kind(text: str) -> str | None:
        return next((kind for kind, rx in words if rx.search(text)), None)

    problems = [
        f"neutral token {t!r} matches {k}"
        for t in neutral_tokens(instance)
        if (k := word_kind(t))
    ]
    own = {
        Kind.WORD_CARD: "card",
        Kind.WORD_PASSWORD: "password",
        Kind.WORD_OTP: "one_time_code",
    }
    for r in instance.regions:
        if r.exposure is not Exposure.FIELD:
            continue
        spec = KINDS[r.plant.kind]
        label = " ".join(
            [spec.words, r.anchor]
            + [v for k, v in spec.attrs if k in ("name", "id", "placeholder", "title")]
        )
        expected = own.get(r.plant.kind)
        if word_kind(label) != expected:
            problems.append(f"{r.id} label {label!r} classifies as {word_kind(label)}")
    if not re.search(rules["codeSlot"]["context"], SPLIT_FORM_TEXT, re.I):
        problems.append("the split code's form text misses codeSlot.context")
    return problems


class Act(StrEnum):
    """Something the page's own script does when the harness says so (via the oracle)."""

    SHOW_PASSWORD = "show_password"  # flip SHOW_PASSWORD fields to type=text
    LATE_PLANT = (
        "late_plant"  # set the late fields and fire input/change, like autofill
    )
    ECHO_TITLE = "echo_title"
    ECHO_URL = "echo_url"
    ALERT = "alert"
    DOWNLOAD = "download"
    POPUP = "popup"  # open a popup whose title shows values
    ADD_LATE_FRAME = "add_late_frame"  # insert #fr-late during control
    REPAINT = "repaint"  # change a counter every 200 ms, so live streams have frames
    # copy layout, before each probe: fields back to their plants, a fresh baseline token
    # focused and selected, an empty paste box
    REBASE = "rebase"


# Acts that need a user gesture (popups and downloads), and the agent event each causes.
GESTURE_ACTS = frozenset({Act.DOWNLOAD, Act.POPUP})
ACT_EVENT: Mapping[Act, str] = {
    Act.ALERT: "dialog",
    Act.DOWNLOAD: "download",
    Act.POPUP: "popup",
}
ACT_EXPOSURE: Mapping[Act, Exposure] = {
    Act.ECHO_TITLE: Exposure.TITLE,
    Act.ECHO_URL: Exposure.URL,
    Act.ALERT: Exposure.DIALOG,
    Act.DOWNLOAD: Exposure.DOWNLOAD,
    Act.POPUP: Exposure.POPUP_TITLE,
}


class Phase(StrEnum):
    """What the SecretSet can hold about an instance's plants when a probe runs."""

    ANY = "any"  # field reads: L1 masks by classification, the set does not matter
    COLD = "cold"  # first agent touch of an instance: nothing ever saw its values
    WARM = "warm"  # after a registration step (host sweep, POST /v1/secrets)
    JOURNALED = "journaled"  # touched, then the page planted with input events
    DURING_CONTROL = "during_control"  # a takeover holds the tab (reads: block)
    CONTROL_RACE = "control_race"  # raw control display, read at once after a key
    CONTROL_SETTLED = "control_settled"  # raw control display, after the debounce
    AFTER_RELEASE = "after_release"  # takeover released or control display closed
    # The agent just typed its own values: claw-sdk holds them, unregistered, until no
    # keystroke came for its quiet period (TYPED_QUIET); a run's reply is masked with them.
    AGENT_TYPED = "agent_typed"
    # The agent wrote the page with setContent: its sweeps alone registered the values.
    AFTER_CONTENT = "after_content"


class PhaseError(AssertionError):
    """A scenario claimed a phase its instance's history cannot support: a harness bug."""


class PhaseGuard:
    """Per instance: which phase a probe may claim, given what already happened to it.

    The Session calls `claim` before every probe and the `note_*` methods after every
    step, so a scenario cannot record a cold read that was not cold. The rule needs no
    model of what a claw-sdk read sweeps (its frame, or its frame's whole site): an
    instance whose values nothing has touched is cold whatever the sweep scope is.
    """

    def __init__(self) -> None:
        self._touched: set[str] = (
            set()
        )  # instances an agent read or a host step reached
        self._registered: set[str] = set()  # instances a registration step covered
        self._journal_pending: set[str] = set()  # late-planted after a touch, unread
        self._control: tuple[str, str] | None = None  # ("takeover"|"display", instance)
        self._released: set[str] = set()
        self._typed: set[str] = (
            set()
        )  # the agent typed into it, nothing registered since
        self._written: set[str] = set()  # the agent wrote the page with setContent

    def claim(self, instance: Instance, phase: Phase, runs: int) -> None:
        name = instance.name
        ok = {
            Phase.ANY: True,
            Phase.COLD: name not in self._touched and runs == 1,
            Phase.WARM: name in self._registered,
            Phase.JOURNALED: name in self._journal_pending and runs == 1,
            Phase.DURING_CONTROL: self._control == ("takeover", name),
            Phase.CONTROL_RACE: self._control == ("display", name),
            Phase.CONTROL_SETTLED: self._control == ("display", name),
            Phase.AFTER_RELEASE: name in self._released,
            Phase.AGENT_TYPED: name in self._typed,
            Phase.AFTER_CONTENT: name in self._written,
        }[phase]
        if not ok:
            raise PhaseError(f"{name}: {phase} does not hold ({runs} runs)")
        self._journal_pending.discard(name)

    def note_touched(self, instance: Instance) -> None:
        self._touched.add(instance.name)

    def note_typed(self, instance: Instance) -> None:
        self._touched.add(instance.name)
        self._typed.add(instance.name)

    def note_written(self, instance: Instance) -> None:
        self._touched.add(instance.name)
        self._written.add(instance.name)

    def note_registered(self, instance: Instance) -> None:
        self._touched.add(instance.name)
        self._registered.add(instance.name)
        self._journal_pending.discard(instance.name)
        self._typed.discard(instance.name)

    def note_late_plant(self, instance: Instance) -> None:
        if instance.name not in self._touched:
            raise PhaseError(f"{instance.name}: late plant before any touch")
        if instance.name not in self._registered:
            self._journal_pending.add(instance.name)

    def note_control(self, kind: str | None, instance: Instance | None = None) -> None:
        if kind is None:
            assert self._control is not None
            self._released.add(self._control[1])
            self._registered.add(self._control[1])
            self._control = None
        else:
            assert instance is not None
            self._touched.add(instance.name)
            self._control = (kind, instance.name)


class Family(StrEnum):
    AGENT_TEXT = "agent_text"  # a run's text result: L1 at the read, L2 at the door
    AGENT_PIXELS = "agent_pixels"  # page.screenshot / page.pdf: L3
    AGENT_SCRIPT = "agent_script"  # main-world script and built-ins: held
    # A copy or cut chord in a field, judged by what the clipboard holds after (D6.3)
    AGENT_KEYS = "agent_keys"
    HOST_RECORD = "host_record"  # trace records: masked once, when written
    HOST_REPLY = "host_reply"  # host control routes: ids and counts only
    LIVE_PIXELS = "live_pixels"  # op screenshots, screencast, video, view display: L3
    CLIPBOARD = "clipboard"  # ServerCutText to any RFB client
    DISK = "disk"  # every byte under /data/browserclaw, the logs
    INFO = "info"  # Chromium's own state: by design, never pass or fail


class ChannelId(StrEnum):
    HARNESS = "harness runs (goto, close)"
    INPUT_VALUE = "locator.inputValue"
    GET_ATTRIBUTE = "locator.getAttribute"
    TEXT_CONTENT = "locator.textContent"
    INNER_TEXT = "locator.innerText"
    INNER_HTML = "locator.innerHTML"
    ALL_TEXT_CONTENTS = "locator.allTextContents"
    ALL_INNER_TEXTS = "locator.allInnerTexts"
    LOCATOR_ARIA = "locator.ariaSnapshot"
    TITLE = "page.title"
    CONTENT = "page.content"
    ARIA = "page.ariaSnapshot"
    ARIA_AI = "page.ariaSnapshot(ai)"
    GET_BY_REF = "page.getByRef"
    HANDLE = "elementHandle.inputValue"
    STRICT_PREVIEW = "strict-mode preview"
    FILL_STRICT = "locator.fill strict preview"
    PRESS_STRICT = "locator.press strict preview"
    FILL_CALL_LOG = "locator.fill call log"
    TO_HAVE_VALUE = "expect.toHaveValue"
    TO_HAVE_ATTRIBUTE = "expect.toHaveAttribute(value)"
    TO_HAVE_TEXT = "expect.toHaveText"
    COPY_CONTROL_C = "keyboard.press Control+C"
    COPY_META_C = "keyboard.press Meta+C"
    COPY_CONTROL_INSERT = "keyboard.press Control+Insert"
    COPY_HELD_CONTROL = "keyboard.down Control, press c"
    CUT_CONTROL_X = "keyboard.press Control+X"
    CUT_SHIFT_DELETE = "keyboard.press Shift+Delete"
    KEYBOARD_TYPE = "keyboard.type"
    FACTS = "page facts"
    DIALOG = "dialog event"
    DOWNLOAD = "download event"
    POPUP = "popup event"
    REPLAY = "requestId replay"
    SCREENSHOT = "page.screenshot"
    PDF = "page.pdf"
    EVALUATE = "page.evaluate"
    OPENER_EVALUATE = "page.evaluate (popup, window.opener)"
    FRAME_EVALUATE = "frame.evaluate"
    WAIT_FOR_FUNCTION = "page.waitForFunction"
    LOCATOR_EVALUATE = "locator.evaluate"
    EVALUATE_ALL = "locator.evaluateAll"
    EVAL_ONE = "page.$eval"
    EVAL_ALL = "page.$$eval"
    HANDLE_EVALUATE = "elementHandle.evaluate"
    SET_CONTENT = "page.setContent"
    UPLOAD = "locator.setInputFiles (path)"
    UPLOAD_BUFFER = "locator.setInputFiles (buffer)"
    UPLOAD_NEO = "locator.setInputFiles (Neo's own file)"
    BRING_TO_FRONT = "page.bringToFront"
    SOLVE_CAPTCHA = "neo.solveCaptcha"
    RUNS = "GET /v1/runs"
    RUN = "GET /v1/runs/{id}"
    OPERATION = "GET /v1/operations/{op}"
    TRACE = "GET /v1/trace"
    EVENTS = "/v1/events (live)"
    EVENTS_REPLAY = "/v1/events (replay)"
    HOST_REPLIES = "takeover/private/sweep/secrets replies"
    OP_SCREENSHOT = "operation screenshots"
    SCREENCAST = "/v1/tabs/{id}/screencast"
    VIDEO = "/v1/tabs/{id}/video"
    DISPLAY_VIEW = "/v1/display view"
    FRONT_VIEW = "/v1/display view (private tab brought forward)"
    CLIPBOARD_VIEW = "clipboard to a view display"
    CLIPBOARD_CONTROL = "clipboard to a control display"
    TRACE_STORE = "trace.db, -wal, -shm"
    STORE_SCREENSHOTS = "screenshots/ on disk"
    CLAW_LOGS = "claw-sdk logs"
    DOCKER_LOGS = "docker logs"
    PROFILE = "/data/profile (info)"


Reaches = Callable[[Region], bool]
CodeTemplate = Callable[[Instance, Sequence[Region], Location | None], str]


@dataclasses.dataclass(frozen=True)
class ChannelSpec:
    """One way a value can leave Neo: how the harness exercises it, what the contract says.

    `reaches` and `refusals` only ever choose between n/a, withheld and inconclusive. They
    cannot hide a leak: a value found in the bytes is LEAKED whatever the spec claims.
    `batch` says how plan_runs splits a read: "location" (one run per frame location),
    "exposure" (fields and echoes apart), "region" (one run each), "page" (one run), or
    "none" (not an agent run). `targets` are the regions a probe asks about when they are
    more than its contract reaches (an action aimed at a field whose error, by contract,
    shows none of it); by default they are `reaches`.
    """

    id: ChannelId
    family: Family
    reaches: Reaches
    code: CodeTemplate | None = None
    batch: str = "none"
    targets: Reaches | None = None
    refusals: frozenset[str] = frozenset()  # "owner/code" answers that mean withheld
    not_built: frozenset[str] = frozenset()  # answers that mean the method is not built
    # How any other refusal reads in its unproven cell's reason, before the owner/code.
    other_refusal: str = "refused"
    # An op that must hand the page what it names: sent, with the page never showing it
    # (the harness's "uploaded" step), it reads unproven with this reason.
    not_received: str | None = None
    ops: frozenset[str] = frozenset()  # ops.generated.ts ops this probe exercises
    reads_fields: bool = False  # in FIELD_READS (scenario "fields")
    reads_echoes: bool = (
        False  # in ECHO_READS (scenario "echoes": a cold instance each)
    )
    # In SCRIPT_READS (scenario "script"): main-world script, each planned run the first
    # touch of a page of its own.
    reads_script: bool = False
    # The probe finds regions by selector, so it never asks for the closed root's. False
    # for a main-world reader, which reaches what the page's own script can, the closed
    # root included, through the page's own reference to it.
    by_selector: bool = True
    after_release: bool = False  # in AFTER_RELEASE_READS
    l2_only: bool = False  # the design masks this exit with L2 only (residual BD-L2)
    # Some answers show only some of the regions they reach (a strict-mode error
    # previews ten elements): an unlisted region is n/a there. `answer_errors` are
    # errors whose message is the probe's answer rather than a refusal.
    lists_some: bool = False
    answer_errors: frozenset[str] = frozenset()
    # The layouts the keys scenario reads this channel over, each on an instance of its
    # own: FIELDS as is, ECHOES cold then warm, LOCKED for actions that write.
    keys: frozenset[Layout] = frozenset()
    # The run puts the value in the page itself (the agent's typing): the proof after it
    # is the value's presence, as an armed run's is.
    types: bool = False
    # The op answers nothing by contract (setContent, setInputFiles, bringToFront): its
    # cells say only whether Neo ran it, refused it by contract or has not built it, for
    # the regions it would expose whether or not the page shows them yet. A value found
    # in its answer is still a leak.
    void: bool = False


def reach(
    *,
    fields: Collection[Location] = (),
    exposures: Collection[Exposure] = (),
    documents: Collection[Location] = (Location.MAIN,),
) -> Reaches:
    """FIELD regions in `fields`; other exposures when they sit in one of `documents`."""

    def predicate(region: Region) -> bool:
        if region.exposure is Exposure.FIELD:
            return region.location in fields
        return region.exposure in exposures and region.location in documents

    return predicate


def nothing(region: Region) -> bool:
    return False


def everything(region: Region) -> bool:
    return True


# Run code. Templates take regions and frame paths and emit ids and selectors only; a
# value or an anchor in the output is a harness bug that assert_clean stops at the wire.
# Keyed reads return {<region id>: {ok: <read>} | {err, owner, code}}, one try each.
_TIMEOUT = "{timeout: 2000}"


def _root(instance: Instance, location: Location | None) -> str:
    if location is Location.POPUP:
        main = json.dumps(f"/fx/{instance.name}/main")
        mine = json.dumps(f"/fx/{instance.name}/")
        return (
            f"context.pages().find((p) => p.url().includes({mine})"
            f" && !p.url().includes({main}))"
        )
    path = PLACES[location].frame_path if location else ()
    return "page" + "".join(f".frameLocator('#{f}')" for f in path)


_KEYED_HEAD = """\
const out = {};
const read = async (id, f) => {
  try { out[id] = {ok: await f(root.locator('#' + id), id)}; }
  catch (e) { out[id] = {err: String(e && e.message || e), owner: e && e.owner, code: e && e.code}; }
};
"""


def keyed(
    read_for: Callable[[Region], str], *, concurrent: bool = False
) -> CodeTemplate:
    """One run per location, one try per region; `read_for(region)` is the JS applied to
    the region's locator `l` (and its `id`), e.g. "l.inputValue({timeout: 2000})".

    `concurrent` starts every try at once: for probes that wait out their timeout on
    purpose (a failing expect, an action on a read-only field), so a location costs one
    timeout, not one per region. The door admits overlapping ops without serializing them
    (claw-sdk dispatch/policy.rs), and each try still answers under its own key."""

    def template(
        instance: Instance, regions: Sequence[Region], location: Location | None
    ) -> str:
        lines = [f"const root = {_root(instance, location)};", _KEYED_HEAD]
        calls = [f"read({json.dumps(r.id)}, (l, id) => {read_for(r)})" for r in regions]
        if concurrent:
            lines.append("await Promise.all([\n  " + ",\n  ".join(calls) + ",\n]);")
        else:
            lines += [f"await {call};" for call in calls]
        return "\n".join([*lines, "return out;"])

    return template


def same(read: str) -> Callable[[Region], str]:
    return lambda region: read


def whole_page(expression: str) -> CodeTemplate:
    """One run returning one page-level read."""

    def template(
        instance: Instance, regions: Sequence[Region], location: Location | None
    ) -> str:
        return f"return await {expression};"

    return template


def on_event(event: str, then: str) -> CodeTemplate:
    """A run armed before the page acts. The Session fires the act once the run's
    events.wait operation has started (seen on /v1/events)."""

    def template(
        instance: Instance, regions: Sequence[Region], location: Location | None
    ) -> str:
        return (
            f"const e = await page.waitForEvent('{event}', {{timeout: 20000}});\n{then}"
        )

    return template


def void_run(ids: Sequence[str], call: str) -> str:
    """The run of an op that answers nothing (ChannelSpec.void): `call`, then {sent:
    true}. A refusal is caught and keyed by each region the op would expose, with its
    owner and code: one the prelude throws itself (a buffer payload today) would reach
    the run as the script's own error, its code lost."""
    keyed = ", ".join(f"{json.dumps(i)}: refused" for i in ids)
    return "\n".join(
        [
            "try {",
            f"  await {call};",
            "  return {sent: true};",
            "} catch (e) {",
            f"  const refused = {_CAUGHT};",
            f"  return {{{keyed}}};",
            "}",
        ]
    )


def bring_to_front(
    instance: Instance, regions: Sequence[Region], location: Location | None
) -> str:
    """page.bringToFront on the run's page: the tab really comes forward."""
    return void_run([r.id for r in regions], "page.bringToFront()")


def content_html(instance: Instance) -> str:
    """The HTML the agent hands page.setContent: the fixture's script, then the top
    document's frames, and no value. The script comes first so it listens before any
    frame reports its values, inside an explicit <body> (one written before any content
    would land in <head>, with no body yet to render into); it renders the echoes and
    titles from the top's slice, whose `written` flag keeps it from adding the frames."""
    frames = "".join(
        f'<iframe id="{f["id"]}" src="{f["src"]}"></iframe>'
        for f in instance.slice(Location.MAIN)["frames"]
    )
    return f'<body><script src="/fx/fx.js"></script>{frames}'


def set_content(
    instance: Instance, regions: Sequence[Region], location: Location | None
) -> str:
    """page.setContent with content_html, resolved once the page and its frames loaded."""
    html = json.dumps(content_html(instance))
    return void_run([r.id for r in regions], f"page.setContent({html})")


# setInputFiles. The owner serves any path (the caller sandboxes), so the agent can hand
# the page a file of Neo's disk, which the page reads and echoes: the oracle writes a
# fresh value into a file, and no request to Neo carries it. One file sits outside Neo's
# state; the other in Neo's downloads dir (entrypoint.sh: $NEO_DATA_DIR/downloads), which
# the deny list must leave alone.
UPLOAD_DIRS: Mapping[Origin, str] = {
    Origin.FILE: "/tmp/leakfx/up",
    Origin.DOWNLOAD: "/data/downloads",
}
# How the deny list refuses an upload of one of Neo's own secret files: its own code,
# with a message that names no path (decision 33b; held-methods.md's addendum gives this
# code as its example). An ordinary file refused with it is a functional regression.
DENY_CODE = "script/neo_secret_upload_refused"


def upload_path(region: Region) -> str:
    """Where the file a FILE or DOWNLOAD plant's input is given sits: named by the
    region's id, in its origin's directory."""
    return f"{UPLOAD_DIRS[region.plant.origin]}/{region.id}.txt"


def upload(
    instance: Instance, regions: Sequence[Region], location: Location | None
) -> str:
    """locator.setInputFiles with the path of the file the oracle wrote: a path, which
    is not a value."""
    (region,) = regions
    path = json.dumps(upload_path(region))
    return void_run([region.id], f"page.locator('#{region.id}').setInputFiles({path})")


def buffer_code(instance: Instance, region: Region) -> str:
    """locator.setInputFiles with a {name, mimeType, buffer} payload of an AGENT plant's
    value: the one run that carries it, its own (as typing_code does), so assert_clean
    admits it in this request alone. Buffer is Playwright's type; the runtime may lack
    it, and then the payload is the value's bytes."""
    assert region.plant.origin is Origin.AGENT and region.exposure is Exposure.FIELD
    payload = "{name: 'upload.txt', mimeType: 'text/plain', buffer}"
    return "\n".join(
        [
            f"const text = {json.dumps(region.expected())};",
            "const buffer = typeof Buffer === 'function' ? Buffer.from(text)"
            " : Uint8Array.from(text, (c) => c.charCodeAt(0));",
            void_run(
                [region.id], f"page.locator('#{region.id}').setInputFiles({payload})"
            ),
        ]
    )


# The deny list (decision 33b): setInputFiles uploads any path as Playwright does except
# Neo's own secret files, which it refuses with DENY_CODE before reading any byte. Each
# probe uploads one into a file input of its own, by its path or by a shape that hides
# the path; the oracle then compares what the page read with the file, by sha256, inside
# the container, so neither the bytes nor a digest of them ever reach the evidence.
class NeoFile(StrEnum):
    """One of Neo's own secret files, which setInputFiles must refuse."""

    HOST_TOKEN = "host token"
    CAPTCHA_KEY = "CAPTCHA key"
    TRACE_DB = "trace.db"
    TRACE_WAL = "trace.db-wal"
    COOKIES = "profile Cookies"
    LOGIN_DATA = "profile Login Data"


# Where each sits in the harness's container: the token and a fake CAPTCHA key where
# leak_wire.Docker.run mounts them (the key in a directory of its own), the trace store
# in claw-sdk's state dir and the credential stores in the browser profile, both under
# NEO_DATA_DIR=/data (entrypoint.sh).
NEO_FILES: Mapping[NeoFile, str] = {
    NeoFile.HOST_TOKEN: "/run/neo-host-token",
    NeoFile.CAPTCHA_KEY: "/run/neo-captcha/capsolver-api-key",
    NeoFile.TRACE_DB: "/data/browserclaw/claw-sdk/trace.db",
    NeoFile.TRACE_WAL: "/data/browserclaw/claw-sdk/trace.db-wal",
    NeoFile.COOKIES: "/data/profile/Default/Cookies",
    NeoFile.LOGIN_DATA: "/data/profile/Default/Login Data",
}


class Via(StrEnum):
    """How a probe names its target. The deny list matches by realpath and by (dev,
    inode), and refuses a directory that holds a denied file, so every shape must fail."""

    PATH = "its path"
    SYMLINK = "a symlink in /tmp"
    DOTDOT = "a path through .."
    HARD_LINK = "a hard link"
    DIRECTORY = "its directory"
    LINK_DIRECTORY = "a directory holding a symlink to it"


@dataclasses.dataclass(frozen=True)
class DenyProbe:
    """One target named one way: the unit of a deny-list verdict."""

    target: NeoFile
    via: Via

    @property
    def directory(self) -> bool:
        """Uploaded as a directory, into an input that takes one (webkitdirectory)."""
        return self.via in (Via.DIRECTORY, Via.LINK_DIRECTORY)

    def __str__(self) -> str:
        return f"{self.target} by {self.via}"


DENY_PROBES: tuple[DenyProbe, ...] = (
    *(DenyProbe(target, Via.PATH) for target in NeoFile),
    DenyProbe(NeoFile.HOST_TOKEN, Via.SYMLINK),
    DenyProbe(NeoFile.HOST_TOKEN, Via.DOTDOT),
    DenyProbe(NeoFile.HOST_TOKEN, Via.LINK_DIRECTORY),
    DenyProbe(NeoFile.TRACE_DB, Via.HARD_LINK),
    DenyProbe(NeoFile.TRACE_DB, Via.DIRECTORY),
)
# Where the oracle makes the links a probe names: on the container's root filesystem,
# which also holds trace.db (the harness mounts no volume on /data), so a hard link to it
# can be made there.
DENY_DIR = "/tmp/leakfx/deny"
# The UPLOAD page's own file input for the read-back's positive control, outside every
# plant: the oracle sets a file of its own there over DevTools, and the comparison that
# judges the probes must find the page read it (fx.js renders it in MAIN).
DIGEST_CONTROL = "fx-digest-control"


@dataclasses.dataclass(frozen=True)
class DenyUpload:
    """One deny-list probe on one UPLOAD page: the file input it uses, the path the agent
    names, and the links the oracle makes first, each (kind, the target, where)."""

    probe: DenyProbe
    region: Region
    named: str
    links: tuple[tuple[str, str, str], ...] = ()

    @property
    def paths(self) -> tuple[str, str]:
        """What the refusal must not name: the path the agent gave and its target's."""
        return (self.named, NEO_FILES[self.probe.target])

    def code(self) -> str:
        """setInputFiles with the named path on the probe's own input: a path, which is
        not a value. A refusal is keyed by the input, so its code survives (void_run)."""
        path = json.dumps(self.named)
        call = f"page.locator('#{self.region.id}').setInputFiles({path})"
        return void_run([self.region.id], call)


def deny_uploads(instance: Instance) -> tuple[DenyUpload, ...]:
    """Every deny-list probe of an UPLOAD page, in DENY_PROBES order: the page plants one
    NEO plant per probe in that order, each with one file input, so a file one probe
    uploaded can never be read as another's."""
    fields = [r for r in instance.regions if r.plant.origin is Origin.NEO]
    if not fields:
        return ()
    out = []
    for probe, region in zip(DENY_PROBES, fields, strict=True):
        target = NEO_FILES[probe.target]
        here = f"{DENY_DIR}/{region.id}"
        inside = f"{here}/{posixpath.basename(target)}"
        named, links = {
            Via.PATH: (target, ()),
            Via.SYMLINK: (here, (("symlink", target, here),)),
            Via.DOTDOT: (f"/tmp/..{target}", ()),
            Via.HARD_LINK: (here, (("hard link", target, here),)),
            Via.DIRECTORY: (posixpath.dirname(target), ()),
            Via.LINK_DIRECTORY: (here, (("symlink", target, inside),)),
        }[probe.via]
        out.append(DenyUpload(probe, region, named, links))
    return tuple(out)


# Main-world reads (design D3's guard). The agent's function runs where the page's own
# script does, so it reaches what the page can: its document, both shadow roots, and
# same-origin frames, also behind a cross-origin one. The guard sweeps the frames the op
# can reach before and after it and masks what it found in the result
# (FieldValues::mask_json): a value of 4+ characters plain, JSON-escaped or
# percent-encoded anywhere in a string, a shorter one only as a whole string. A probe
# asks for exactly those spellings, so a raw answer is a leak and never the deliberate
# encoding the threat model leaves out (base64, a short value inside longer text).
LONG_SPELLINGS = ("plain", "json", "uri")
SHORT_SPELLINGS = ("plain",)


def spellings(region: Region) -> tuple[str, ...]:
    return SHORT_SPELLINGS if region.short else LONG_SPELLINGS


# How the top document's main world reaches each location it can, as steps the reader
# walks: "frame" an iframe element by id, "child" a window's frames[i] (which a
# cross-origin window still exposes), "shadow" an open root by its host's id, "closed" the
# closed root through the page's own reference to it (fx.js keeps it in FX.closed). Only
# documents of the top's origin: reading a cross-origin frame's document throws.
MAIN_WORLD_STEPS: Mapping[Location, tuple[tuple[str, str | int | None], ...]] = {
    Location.MAIN: (),
    Location.SHADOW_OPEN: (("shadow", "host-open"),),
    Location.SHADOW_CLOSED: (("closed", None),),
    Location.SAME_ORIGIN: (("frame", "fr-so"),),
    Location.NESTED: (("frame", "fr-xo"), ("child", 0)),
}
# A popup reaches the same through window.opener, but for the closed root: FX is a
# binding of the page's realm, not a property of its window.
OPENER_FIELDS = tuple(
    loc for loc in MAIN_WORLD_STEPS if loc is not Location.SHADOW_CLOSED
)
# frame.evaluate runs in every frame of the matrix but the top (page.evaluate is that
# one), reading the frame's own fields.
FRAME_DOCUMENTS = (
    Location.SAME_ORIGIN,
    Location.SAME_SITE,
    Location.CROSS_SITE,
    Location.NESTED,
)

# The readers run in the page, so they hold nothing from the run's scope. Each returns a
# region's value (a field) or text (an echo) in the spellings asked; `near` is a field's
# label, whose anchor proves the read reached that field (run code names ids only).
_SPELL = (
    "const spell = {plain: (v) => v, json: (v) => JSON.stringify(v),"
    " uri: (v) => encodeURIComponent(v)};"
)
# For page.evaluate and its relatives: [[id, steps, spellings], ...] -> {spelling: {id: {ok}}}.
_MAIN_WORLD_READER = f"""\
(want) => {{
  {_SPELL}
  // A window's document is read only when a step needs it: a cross-origin window on the
  // way (b.test around the nested a.test frame) throws on .document, not on .frames.
  const rootOf = (steps) => {{
    let win = window;
    let root = null;
    const doc = () => root || win.document;
    for (const [step, arg] of steps) {{
      if (step === 'opener') {{ win = win.opener; root = null; }}
      else if (step === 'frame') {{ win = doc().getElementById(arg).contentWindow; root = null; }}
      else if (step === 'child') {{ win = win.frames[arg]; root = null; }}
      else if (step === 'shadow') {{ root = doc().getElementById(arg).shadowRoot; }}
      else if (step === 'closed') {{ root = FX.closed; }}
    }}
    return doc();
  }};
  const out = {{plain: {{}}, json: {{}}, uri: {{}}}};
  for (const [id, steps, spellings] of want) {{
    try {{
      const el = rootOf(steps).getElementById(id);
      const text = el.tagName === 'INPUT' ? el.value : el.textContent;
      for (const s of spellings) out[s][id] = {{ok: spell[s](text)}};
      const label = el.closest('label');
      out.plain[id].near = label ? label.textContent : null;
    }} catch (e) {{
      out.plain[id] = {{err: String(e && e.message || e)}};
    }}
  }}
  return out;
}}"""
# For locator.evaluate, $eval and elementHandle.evaluate: (element, spellings) -> {near, <spelling>}.
_ELEMENT_READER = f"""\
(el, spellings) => {{
  {_SPELL}
  const text = el.tagName === 'INPUT' ? el.value : el.textContent;
  const label = el.closest('label');
  const out = {{near: label ? label.textContent : null}};
  for (const s of spellings) out[s] = spell[s](text);
  return out;
}}"""
# For locator.evaluateAll and $$eval: (elements, {id: spellings}) -> [[id, element answer]].
_ALL_READER = (
    "(els, want) => els.filter((el) => el.id in want)"
    f".map((el) => [el.id, ({_ELEMENT_READER})(el, want[el.id])])"
)
# The run side of the element scripts: each element's answer regrouped into the readers'
# shape, and an op's refusal kept under its region with its owner and code.
_GROUPED_HEAD = """\
const out = {plain: {}, json: {}, uri: {}};
const put = (id, r) => {
  out.plain[id] = {ok: r.plain, near: r.near};
  for (const s of ['json', 'uri']) if (s in r) out[s][id] = {ok: r[s]};
};
const fail = (id, e) => {
  out.plain[id] = {err: String(e && e.message || e), owner: e && e.owner, code: e && e.code};
};"""


def _handle(instance: Instance, location: Location | None, handle: str) -> str:
    """`const root` (the location's frameLocator chain) or `const frame` (its frame, for
    the frame forms: $eval, $$eval, $, evaluate; the page itself for the top document).

    A run's page lists only the frames its last navigation or wait reported, and a fresh
    run has had neither (claw-sdk's prelude caches the tree; upstream lists it live). A
    waitForURL on the URL the page is at already returns at once and refreshes it."""
    if handle == "root":
        return f"const root = {_root(instance, location)};"
    if location in (None, Location.MAIN, Location.SHADOW_OPEN):
        return "const frame = page;"
    url = json.dumps(instance.url(location))
    return (
        f"await page.waitForURL({json.dumps(instance.url())});\n"
        f"const frame = page.frames().find((f) => f.url() === {url});"
    )


def _read(head: Sequence[str], want: object, reader: str, call: str) -> str:
    return "\n".join(
        [
            *head,
            f"const WANT = {json.dumps(want)};",
            f"const READ = {reader};",
            f"return await {call};",
        ]
    )


def top_read(call: str) -> CodeTemplate:
    """A main-world read from the top document (page.evaluate, waitForFunction): `call`
    applies READ to WANT and resolves to READ's answer."""

    def template(
        instance: Instance, regions: Sequence[Region], location: Location | None
    ) -> str:
        want = [[r.id, MAIN_WORLD_STEPS[r.location], spellings(r)] for r in regions]
        return _read([], want, _MAIN_WORLD_READER, call)

    return template


def opener_read(
    instance: Instance, regions: Sequence[Region], location: Location | None
) -> str:
    """page.evaluate in the popup the page opened, reading the page through
    window.opener: the guard must sweep the tabs an opener links (fx-g-opener)."""
    opener = (("opener", None),)
    want = [
        [r.id, (*opener, *MAIN_WORLD_STEPS[r.location]), spellings(r)] for r in regions
    ]
    head = [f"const popup = {_root(instance, Location.POPUP)};"]
    return _read(head, want, _MAIN_WORLD_READER, "popup.evaluate(READ, WANT)")


def frame_read(
    instance: Instance, regions: Sequence[Region], location: Location | None
) -> str:
    """frame.evaluate in one frame of the matrix, reading that frame's own fields."""
    want = [[r.id, (), spellings(r)] for r in regions]
    head = [_handle(instance, location, "frame")]
    return _read(head, want, _MAIN_WORLD_READER, "frame.evaluate(READ, WANT)")


def element_read(handle: str, call: str) -> CodeTemplate:
    """locator.evaluate and its relatives on each field of one location, one call per
    element, each caught on its own: `call` applies READ to the element `id` names with
    its `spellings`, through `root` or `frame` (`handle`)."""

    def template(
        instance: Instance, regions: Sequence[Region], location: Location | None
    ) -> str:
        lines = [
            _handle(instance, location, handle),
            f"const READ = {_ELEMENT_READER};",
            _GROUPED_HEAD,
        ]
        for r in regions:
            ident = json.dumps(r.id)
            lines.append(
                f"try {{ const id = {ident}, spellings = {json.dumps(spellings(r))};"
                f" put(id, await {call}); }} catch (e) {{ fail({ident}, e); }}"
            )
        return "\n".join([*lines, "return out;"])

    return template


def all_read(handle: str, call: str) -> CodeTemplate:
    """locator.evaluateAll and $$eval: one call over every input of one location, READ
    keeping the fields WANT names (id -> spellings)."""

    def template(
        instance: Instance, regions: Sequence[Region], location: Location | None
    ) -> str:
        want = {r.id: spellings(r) for r in regions}
        return "\n".join(
            [
                _handle(instance, location, handle),
                f"const WANT = {json.dumps(want)};",
                f"const READ = {_ALL_READER};",
                _GROUPED_HEAD,
                f"try {{ for (const [id, r] of await {call}) put(id, r); }}",
                "catch (e) { for (const id of Object.keys(WANT)) fail(id, e); }",
                "return out;",
            ]
        )

    return template


def strict_preview(
    instance: Instance, regions: Sequence[Region], location: Location | None
) -> str:
    """An uncaught strict-mode violation: the run's error carries Playwright's element
    previews. Echo regions get the echo variant, fields the input variant."""
    selector = "p.echo" if regions[0].exposure in ECHO_LIKE else "input"
    read = "textContent" if selector == "p.echo" else "inputValue"
    return f"return await page.locator('{selector}').{read}({_TIMEOUT});"


# How long a probe that must fail waits: an expect that never matches, an action on a
# read-only field. Long enough for a few polls, so a call log shows its repeated lines.
_FAILS_AFTER = "{timeout: 800}"


def strict_action(call: str) -> CodeTemplate:
    """An action's uncaught strict-mode violation: `call` (e.g. "fill('q')") on every
    input or on every echo. It fails at resolution, before anything is sent, and its error
    previews up to ten matches: markup with value attributes, or an echo's text."""

    def template(
        instance: Instance, regions: Sequence[Region], location: Location | None
    ) -> str:
        selector = "p.echo" if regions[0].exposure in ECHO_LIKE else "input"
        return f"return await page.locator('{selector}').{call};"

    return template


_CAUGHT = "{err: String(e && e.message || e), owner: e && e.owner, code: e && e.code}"

# A copy probe's positive control: Control+C on the plain baseline field the page has just
# focused with a fresh token selected. It must be sent, or the probe proves nothing.
COPY_BASELINE_CODE = "await page.keyboard.press('Control+C');\nreturn 1;"


def chord_probe(chord: str) -> CodeTemplate:
    """A copy probe's second run: `chord` (JS statements) in the field the page focused
    with its value selected, then a paste into the paste box, whose digest the oracle
    reads after. Both are caught, so the run always answers the chord's outcome; the paste
    box shows what the chord left on the clipboard: the baseline (nothing copied) or the
    field's value (laundered). The answers are not keyed by the field's id: that shape
    means a read of the field, and the judge would take "sent" for its value."""

    def template(
        instance: Instance, regions: Sequence[Region], location: Location | None
    ) -> str:
        return "\n".join(
            [
                "const out = {};",
                f"try {{ {chord}; out.chord = {{sent: true}}; }}",
                f"catch (e) {{ out.chord = {_CAUGHT}; }}",
                "try {",
                f"  await page.locator('#{COPY_DROP}').focus({_TIMEOUT});",
                "  await page.keyboard.press('Control+V');",
                "  out.paste = {sent: true};",
                f"}} catch (e) {{ out.paste = {_CAUGHT}; }}",
                "return out;",
            ]
        )

    return template


def typing_code(instance: Instance, region: Region) -> str:
    """The run that types one box of an AGENT plant: the only run code that carries a
    planted value, its own, so assert_clean admits it in this request alone (`inject`).

    Where a locator reaches the field it focuses it, types, and reads the field back keyed
    in the same run; the closed root is focused by the page first and only typed into.
    `id` names the region beside the value, so the report's redactor can tell a short
    value's slot in this code when a leak block prints it."""
    assert region.plant.origin is Origin.AGENT and region.exposure is Exposure.FIELD
    head = f"const id = {json.dumps(region.id)};"
    typed = f"await page.keyboard.type({json.dumps(region.expected())});"
    if not PLACES[region.location].agent_reachable:
        return f"{head}\n{typed}\nreturn {{}};"
    return "\n".join(
        [
            head,
            f"const root = {_root(instance, region.location)};",
            "const l = root.locator('#' + id);",
            f"await l.focus({_TIMEOUT});",
            typed,
            "const out = {};",
            f"try {{ out[id] = {{ok: await l.inputValue({_TIMEOUT})}}; }}",
            f"catch (e) {{ out[id] = {_CAUGHT}; }}",
            "return out;",
        ]
    )


def by_ref(
    instance: Instance, regions: Sequence[Region], location: Location | None
) -> str:
    """The ai snapshot, then getByRef(ref).inputValue() for every textbox ref. The judge
    maps a ref to its region by the anchor in the snapshot line the page produced."""
    return f"""\
const snap = await page.ariaSnapshot({{mode: 'ai'}});
const out = {{}};
for (const line of snap.split('\\n')) {{
  const m = line.match(/- textbox .*\\[ref=([^\\]]+)\\]/);
  if (!m) continue;
  try {{ out[m[1]] = {{line, ok: await page.getByRef(m[1]).inputValue({_TIMEOUT})}}; }}
  catch (e) {{ out[m[1]] = {{line, err: String(e && e.message || e), owner: e && e.owner, code: e && e.code}}; }}
}}
return out;"""


def facts(
    instance: Instance, regions: Sequence[Region], location: Location | None
) -> str:
    """Core's cached page facts: page.url(), and every page's url() and _title."""
    return (
        "return {url: page.url(), title: page._title, "
        "pages: context.pages().map((p) => ({url: p.url(), title: p._title}))};"
    )


ECHO_TEXT = (Exposure.ECHO, Exposure.SERVER_ECHO)
TEXT_DOCUMENTS = (Location.MAIN, *NAV_FRAMES, Location.POPUP)
OWNER_FIELDS = (*AGENT_FIELDS, *NAV_FRAMES, Location.POPUP)
READ_REFUSALS = frozenset({"host/owner_has_control", "neo/unsupported"})
HELD = frozenset({"script/method_unavailable"})
NOT_BUILT = frozenset({"neo/internal_error"})
PIXEL_REFUSALS = frozenset({"host/page_private"})
MAIN_FIELDS = reach(fields=(Location.MAIN,))
# Errors that are the probe's answer: the message is what is judged, not a refusal.
STRICT = frozenset({"script/strict_mode_violation"})
EXPECT_FAILED = frozenset({"website/assertion_failed"})
# The contract for a copy or cut chord in a sensitive field, or where Neo cannot tell
# (a closed shadow root): refused, nothing sent (browseros-playwright copy_refusal.rs).
COPY_REFUSED = frozenset({"script/sensitive_copy_refused"})
_COPY_OPS = frozenset({"keyboard.press", "locator.focus"})
# A main-world method the prelude itself refuses (neo/unsupported) is not built yet.
SCRIPT_NOT_BUILT = NOT_BUILT | {"neo/unsupported"}
# page.evaluate and waitForFunction reach what the top document's own script can.
_TOP_WORLD = reach(fields=tuple(MAIN_WORLD_STEPS), exposures=(Exposure.ECHO,))


def _expect(matcher: str) -> CodeTemplate:
    """`expect(l).<matcher>` (e.g. "toHaveValue('x')", a wrong expectation) with a short
    timeout, all of a location's at once: each fails, and its message prints the Received
    value and the polls' call log."""
    call = f"expect(l).{matcher.removesuffix(')')}, {_FAILS_AFTER})"
    return keyed(same(call), concurrent=True)


def _attribute(region: Region) -> str:
    name = (
        "value" if region.exposure in (Exposure.FIELD, Exposure.PASTED) else "data-echo"
    )
    return f"l.getAttribute('{name}', {_TIMEOUT})"


def _inner_html(region: Region) -> str:
    if region.exposure is Exposure.FIELD:  # the field's container: its value attribute
        return f"l.locator('xpath=..').innerHTML({_TIMEOUT})"
    return f"l.innerHTML({_TIMEOUT})"


_A, _P, _S = Family.AGENT_TEXT, Family.AGENT_PIXELS, Family.AGENT_SCRIPT
_K = Family.AGENT_KEYS
_FIELD_AND_ECHO = reach(fields=AGENT_FIELDS, exposures=(Exposure.ECHO,))
SPECS: Mapping[ChannelId, ChannelSpec] = {
    s.id: s
    for s in (
        ChannelSpec(ChannelId.HARNESS, _A, nothing),
        ChannelSpec(
            ChannelId.INPUT_VALUE,
            _A,
            reach(fields=OWNER_FIELDS, exposures=(Exposure.PASTED,)),
            code=keyed(same(f"l.inputValue({_TIMEOUT})")),
            batch="location",
            refusals=READ_REFUSALS,
            ops=frozenset({"locator.inputValue"}),
            reads_fields=True,
            after_release=True,
        ),
        ChannelSpec(
            ChannelId.GET_ATTRIBUTE,
            _A,
            reach(fields=OWNER_FIELDS, exposures=(Exposure.ECHO, Exposure.PASTED)),
            code=keyed(_attribute),
            batch="location",
            refusals=READ_REFUSALS,
            ops=frozenset({"locator.getAttribute"}),
            reads_fields=True,
            reads_echoes=True,
        ),
        ChannelSpec(
            ChannelId.TEXT_CONTENT,
            _A,
            reach(exposures=ECHO_TEXT, documents=TEXT_DOCUMENTS),
            code=keyed(same(f"l.textContent({_TIMEOUT})")),
            batch="location",
            refusals=READ_REFUSALS,
            ops=frozenset({"locator.textContent"}),
            reads_echoes=True,
            after_release=True,
        ),
        ChannelSpec(
            ChannelId.INNER_TEXT,
            _A,
            reach(exposures=ECHO_TEXT, documents=TEXT_DOCUMENTS),
            code=keyed(same(f"l.innerText({_TIMEOUT})")),
            batch="location",
            refusals=READ_REFUSALS,
            ops=frozenset({"locator.innerText"}),
            reads_echoes=True,
        ),
        ChannelSpec(
            ChannelId.INNER_HTML,
            _A,
            reach(fields=AGENT_FIELDS, exposures=(Exposure.ECHO,)),
            code=keyed(_inner_html),
            batch="location",
            refusals=READ_REFUSALS,
            ops=frozenset({"locator.innerHTML"}),
            reads_fields=True,
            reads_echoes=True,
        ),
        ChannelSpec(
            ChannelId.ALL_TEXT_CONTENTS,
            _A,
            reach(exposures=(Exposure.ECHO,)),
            code=whole_page("page.locator('p.echo').allTextContents()"),
            batch="page",
            refusals=READ_REFUSALS,
            ops=frozenset({"locator.allTextContents"}),
            reads_echoes=True,
        ),
        ChannelSpec(
            ChannelId.ALL_INNER_TEXTS,
            _A,
            reach(exposures=(Exposure.ECHO,)),
            code=whole_page("page.locator('p.echo').allInnerTexts()"),
            batch="page",
            refusals=READ_REFUSALS,
            ops=frozenset({"locator.allInnerTexts"}),
            reads_echoes=True,
        ),
        ChannelSpec(
            ChannelId.LOCATOR_ARIA,
            _A,
            reach(
                fields=(Location.MAIN, Location.SHADOW_OPEN),
                exposures=(Exposure.ECHO, Exposure.PASTED),
            ),
            code=whole_page(f"page.locator('body').ariaSnapshot({_TIMEOUT})"),
            batch="page",
            refusals=READ_REFUSALS,
            ops=frozenset({"locator.ariaSnapshot"}),
            reads_fields=True,
            reads_echoes=True,
        ),
        ChannelSpec(
            ChannelId.TITLE,
            _A,
            reach(exposures=(Exposure.TITLE,)),
            code=whole_page("page.title()"),
            batch="page",
            refusals=READ_REFUSALS,
            ops=frozenset({"page.title"}),
        ),
        ChannelSpec(
            ChannelId.CONTENT,
            _A,
            reach(
                fields=(Location.MAIN,),
                exposures=(Exposure.ECHO, Exposure.TITLE, Exposure.PASTED),
            ),
            code=whole_page("page.content()"),
            batch="page",
            refusals=READ_REFUSALS,
            ops=frozenset({"page.content"}),
            reads_fields=True,
            reads_echoes=True,
        ),
        ChannelSpec(
            ChannelId.ARIA,
            _A,
            reach(
                fields=(Location.MAIN, Location.SHADOW_OPEN),
                exposures=(Exposure.ECHO, Exposure.PASTED),
            ),
            code=whole_page("page.ariaSnapshot()"),
            batch="page",
            refusals=READ_REFUSALS,
            ops=frozenset({"page.ariaSnapshot"}),
            reads_fields=True,
            reads_echoes=True,
        ),
        ChannelSpec(
            ChannelId.ARIA_AI,
            _A,
            reach(
                fields=(*AGENT_FIELDS, *NAV_FRAMES),
                exposures=(*ECHO_TEXT, Exposure.PASTED),
                documents=(Location.MAIN, *NAV_FRAMES),
            ),
            code=whole_page("page.ariaSnapshot({mode: 'ai'})"),
            batch="page",
            refusals=READ_REFUSALS,
            ops=frozenset({"page.ariaSnapshot"}),
            reads_fields=True,
            reads_echoes=True,
            after_release=True,
        ),
        ChannelSpec(
            ChannelId.GET_BY_REF,
            _A,
            reach(fields=(*AGENT_FIELDS, *NAV_FRAMES), exposures=(Exposure.PASTED,)),
            code=by_ref,
            batch="page",
            refusals=READ_REFUSALS,
            ops=frozenset({"page.ariaSnapshot", "locator.inputValue"}),
            reads_fields=True,
        ),
        ChannelSpec(
            ChannelId.HANDLE,
            _A,
            reach(fields=(Location.MAIN, Location.SHADOW_OPEN)),
            code=keyed(same("page.$('#' + id).then((h) => h.inputValue())")),
            batch="location",
            refusals=READ_REFUSALS,
            not_built=NOT_BUILT,
            ops=frozenset({"locator.elementHandle", "locator.inputValue"}),
            reads_fields=True,
        ),
        ChannelSpec(
            ChannelId.STRICT_PREVIEW,
            _A,
            reach(
                fields=(Location.MAIN, Location.SHADOW_OPEN), exposures=(Exposure.ECHO,)
            ),
            code=strict_preview,
            batch="exposure",
            lists_some=True,  # a strict-mode error previews ten elements
            answer_errors=frozenset({"script/strict_mode_violation"}),
            ops=frozenset({"locator.textContent", "locator.inputValue"}),
            reads_fields=True,
            reads_echoes=True,
        ),
        ChannelSpec(
            ChannelId.FILL_STRICT,
            _A,
            reach(fields=(Location.MAIN, Location.SHADOW_OPEN)),
            code=strict_action(f"fill('q', {_FAILS_AFTER})"),
            batch="exposure",
            lists_some=True,
            answer_errors=STRICT,
            ops=frozenset({"locator.fill"}),
            keys=frozenset({Layout.LOCKED}),
        ),
        ChannelSpec(
            ChannelId.PRESS_STRICT,
            _A,
            reach(
                fields=(Location.MAIN, Location.SHADOW_OPEN), exposures=(Exposure.ECHO,)
            ),
            code=strict_action(f"press('a', {_FAILS_AFTER})"),
            batch="exposure",
            lists_some=True,
            answer_errors=STRICT,
            ops=frozenset({"locator.press"}),
            keys=frozenset({Layout.LOCKED, Layout.ECHOES}),
        ),
        ChannelSpec(
            # fill on a read-only field waits for it to be editable and times out. Its
            # call log names the locator and the states it waited for, and by contract no
            # element: the crate's actions log no `locator resolved to <preview>` line
            # (form_action.rs), unlike upstream. So it reaches nothing; a preview that
            # does show up gives the field coverage and is judged.
            ChannelId.FILL_CALL_LOG,
            _A,
            nothing,
            code=keyed(same(f"l.fill('q', {_FAILS_AFTER})"), concurrent=True),
            batch="location",
            targets=reach(fields=AGENT_FIELDS),
            answer_errors=frozenset({"website/not_actionable"}),
            ops=frozenset({"locator.fill"}),
            keys=frozenset({Layout.LOCKED}),
        ),
        ChannelSpec(
            ChannelId.TO_HAVE_VALUE,
            _A,
            reach(fields=AGENT_FIELDS),
            code=_expect("toHaveValue('x')"),
            batch="location",
            not_built=NOT_BUILT,
            answer_errors=EXPECT_FAILED,
            ops=frozenset({"expect.locator"}),
            keys=frozenset({Layout.FIELDS}),
        ),
        ChannelSpec(
            # Every field carries its value in a value attribute (fx.js mirrors it, as
            # React does), which the sealed world compares and reports masked.
            ChannelId.TO_HAVE_ATTRIBUTE,
            _A,
            reach(fields=AGENT_FIELDS),
            code=_expect("toHaveAttribute('value', 'x')"),
            batch="location",
            answer_errors=EXPECT_FAILED,
            ops=frozenset({"expect.locator"}),
            keys=frozenset({Layout.FIELDS}),
        ),
        ChannelSpec(
            ChannelId.TO_HAVE_TEXT,
            _A,
            reach(exposures=ECHO_TEXT, documents=TEXT_DOCUMENTS),
            code=_expect("toHaveText('x')"),
            batch="location",
            answer_errors=EXPECT_FAILED,
            ops=frozenset({"expect.locator"}),
            keys=frozenset({Layout.ECHOES}),
        ),
        # Copy and cut (D6.3), one row per chord, in the order each field gets them: the
        # cuts last, since a cut that goes through empties the field. The page focuses the
        # field (the closed root through its closure), so these reach the closed root too.
        # Meta+C is the Mac chord; Neo's keyboard follows the browser's platform, so on
        # Linux it is no copy chord and is sent: what it copies is measured, not assumed.
        ChannelSpec(
            ChannelId.COPY_CONTROL_C,
            _K,
            reach(fields=FRAME_MATRIX),
            code=chord_probe("await page.keyboard.press('Control+C')"),
            batch="region",
            refusals=COPY_REFUSED,
            ops=_COPY_OPS,
        ),
        ChannelSpec(
            ChannelId.COPY_META_C,
            _K,
            reach(fields=FRAME_MATRIX),
            code=chord_probe("await page.keyboard.press('Meta+C')"),
            batch="region",
            refusals=COPY_REFUSED,
            ops=_COPY_OPS,
        ),
        ChannelSpec(
            ChannelId.COPY_CONTROL_INSERT,
            _K,
            MAIN_FIELDS,
            code=chord_probe("await page.keyboard.press('Control+Insert')"),
            batch="region",
            refusals=COPY_REFUSED,
            ops=_COPY_OPS,
        ),
        ChannelSpec(
            # Control held down by an earlier op of the run, then a bare c: the copy
            # check must read the keyboard's held modifiers, not the press alone.
            ChannelId.COPY_HELD_CONTROL,
            _K,
            MAIN_FIELDS,
            code=chord_probe(
                "await page.keyboard.down('Control'); "
                "try { await page.keyboard.press('c'); } "
                "finally { await page.keyboard.up('Control'); }"
            ),
            batch="region",
            refusals=COPY_REFUSED,
            ops=_COPY_OPS | {"keyboard.down", "keyboard.up"},
        ),
        ChannelSpec(
            ChannelId.CUT_CONTROL_X,
            _K,
            reach(fields=FRAME_MATRIX),
            code=chord_probe("await page.keyboard.press('Control+X')"),
            batch="region",
            refusals=COPY_REFUSED,
            ops=_COPY_OPS,
        ),
        ChannelSpec(
            ChannelId.CUT_SHIFT_DELETE,
            _K,
            MAIN_FIELDS,
            code=chord_probe("await page.keyboard.press('Shift+Delete')"),
            batch="region",
            refusals=COPY_REFUSED,
            ops=_COPY_OPS,
        ),
        ChannelSpec(
            # The typing run's own reply: it reads the field back keyed. Its code is the
            # declared way in (typing_code), so the Session builds it, not plan_runs.
            ChannelId.KEYBOARD_TYPE,
            _A,
            reach(fields=AGENT_FIELDS),
            ops=frozenset({"keyboard.type", "locator.focus", "locator.inputValue"}),
            types=True,
        ),
        ChannelSpec(
            ChannelId.FACTS,
            _A,
            reach(
                exposures=(Exposure.TITLE, Exposure.URL, Exposure.POPUP_TITLE),
                documents=(Location.MAIN, Location.POPUP),
            ),
            code=facts,
            batch="page",
            ops=frozenset({"context.pages"}),
            l2_only=True,
        ),
        ChannelSpec(
            ChannelId.DIALOG,
            _A,
            reach(exposures=(Exposure.DIALOG,)),
            code=on_event(
                "dialog",
                "const m = e.message(); await e.dismiss(); return {message: m};",
            ),
            batch="page",
            ops=frozenset({"events.wait", "dialog.dismiss"}),
            l2_only=True,
        ),
        ChannelSpec(
            ChannelId.DOWNLOAD,
            _A,
            reach(exposures=(Exposure.DOWNLOAD,)),
            code=on_event(
                "download",
                # path() is refused by contract (neo/unsupported, not in v1): the name is
                # the exit, and the refusal's message is searched like any answer.
                "let path;\n"
                "try { path = await e.path(); } catch (x) { path = String(x.code || x); }\n"
                "return {name: e.suggestedFilename(), path};",
            ),
            batch="page",
            ops=frozenset({"events.wait", "download.path"}),
            l2_only=True,
        ),
        ChannelSpec(
            ChannelId.POPUP,
            _A,
            reach(exposures=(Exposure.POPUP_TITLE,), documents=(Location.POPUP,)),
            code=on_event(
                "popup",
                "await e.waitForLoadState();\n"
                "return {url: e.url(), title: await e.title()};",
            ),
            batch="page",
            ops=frozenset({"events.wait", "page.waitForLoadState"}),
            l2_only=True,
        ),
        ChannelSpec(ChannelId.REPLAY, _A, everything),
        ChannelSpec(
            ChannelId.SCREENSHOT,
            _P,
            MAIN_FIELDS,
            code=whole_page(
                "page.screenshot({type: 'jpeg', quality: 40,"
                " clip: {x: 0, y: 0, width: 640, height: 240}})"
            ),
            batch="page",
            refusals=PIXEL_REFUSALS,
            ops=frozenset({"page.screenshot"}),
        ),
        ChannelSpec(
            ChannelId.PDF,
            _P,
            MAIN_FIELDS,
            code=whole_page("page.pdf()"),
            batch="page",
            refusals=PIXEL_REFUSALS,
            ops=frozenset({"page.pdf"}),
        ),
        # The main-world family: a cold page per planned run (scenario "script"). Held
        # today (script/method_unavailable, EVAL-HELD); served, masked by the guard.
        ChannelSpec(
            ChannelId.EVALUATE,
            _S,
            _TOP_WORLD,
            code=top_read("page.evaluate(READ, WANT)"),
            batch="page",
            refusals=HELD,
            not_built=SCRIPT_NOT_BUILT,
            ops=frozenset({"page.evaluate"}),
            reads_script=True,
            by_selector=False,
        ),
        ChannelSpec(
            ChannelId.OPENER_EVALUATE,
            _S,
            reach(fields=OPENER_FIELDS, exposures=(Exposure.ECHO,)),
            code=opener_read,
            batch="page",
            refusals=HELD,
            not_built=SCRIPT_NOT_BUILT,
            ops=frozenset({"page.evaluate", "context.pages"}),
            by_selector=False,
        ),
        ChannelSpec(
            ChannelId.FRAME_EVALUATE,
            _S,
            reach(fields=FRAME_DOCUMENTS),
            code=frame_read,
            batch="location",
            refusals=HELD,
            not_built=SCRIPT_NOT_BUILT,
            ops=frozenset({"page.evaluate", "page.frames"}),
            reads_script=True,
            by_selector=False,
        ),
        ChannelSpec(
            # Playwright resolves a JSHandle to the truthy value; Neo's holds the value
            # itself, and jsonValue() returns it (prelude ValueHandle).
            ChannelId.WAIT_FOR_FUNCTION,
            _S,
            _TOP_WORLD,
            code=top_read(
                "page.waitForFunction(READ, WANT, {timeout: 5000})"
                ".then((h) => h.jsonValue())"
            ),
            batch="page",
            refusals=HELD,
            not_built=SCRIPT_NOT_BUILT,
            ops=frozenset({"page.waitForFunction"}),
            reads_script=True,
            by_selector=False,
        ),
        ChannelSpec(
            ChannelId.LOCATOR_EVALUATE,
            _S,
            reach(fields=AGENT_FIELDS),
            code=element_read(
                "root", "root.locator('#' + id).evaluate(READ, spellings)"
            ),
            batch="location",
            refusals=HELD,
            not_built=SCRIPT_NOT_BUILT,
            ops=frozenset({"locator.evaluate"}),
            reads_script=True,
        ),
        ChannelSpec(
            ChannelId.EVALUATE_ALL,
            _S,
            reach(fields=AGENT_FIELDS),
            code=all_read("root", "root.locator('input').evaluateAll(READ, WANT)"),
            batch="location",
            refusals=HELD,
            not_built=SCRIPT_NOT_BUILT,
            ops=frozenset({"locator.evaluateAll"}),
            reads_script=True,
        ),
        ChannelSpec(
            ChannelId.EVAL_ONE,
            _S,
            reach(fields=AGENT_FIELDS),
            code=element_read("frame", "frame.$eval('#' + id, READ, spellings)"),
            batch="location",
            refusals=HELD,
            not_built=SCRIPT_NOT_BUILT,
            ops=frozenset({"locator.evaluate", "page.frames"}),
            reads_script=True,
        ),
        ChannelSpec(
            ChannelId.EVAL_ALL,
            _S,
            reach(fields=AGENT_FIELDS),
            code=all_read("frame", "frame.$$eval('input', READ, WANT)"),
            batch="location",
            refusals=HELD,
            not_built=SCRIPT_NOT_BUILT,
            ops=frozenset({"locator.evaluateAll", "page.frames"}),
            reads_script=True,
        ),
        ChannelSpec(
            ChannelId.HANDLE_EVALUATE,
            _S,
            reach(fields=AGENT_FIELDS),
            code=element_read(
                "frame", "frame.$('#' + id).then((h) => h.evaluate(READ, spellings))"
            ),
            batch="location",
            refusals=HELD,
            not_built=SCRIPT_NOT_BUILT,
            ops=frozenset({"locator.elementHandle", "locator.evaluate", "page.frames"}),
            reads_script=True,
        ),
        ChannelSpec(
            # Its HTML's scripts run in the main world, between the guard's sweeps of the
            # page's site (scenario "content"); the reads of what it wrote are judged.
            ChannelId.SET_CONTENT,
            _S,
            reach(exposures=(Exposure.ECHO, Exposure.TITLE)),
            code=set_content,
            batch="page",
            refusals=HELD,
            not_built=SCRIPT_NOT_BUILT,
            ops=frozenset({"page.setContent"}),
            void=True,
        ),
        # setInputFiles answers nothing; the page's echo of what it uploaded is read
        # through textContent (scenario "upload"). Held today (UPLOAD-HELD). These files
        # are ordinary: refusing one is a functional regression, unproven, never a leak.
        ChannelSpec(
            ChannelId.UPLOAD,
            _A,
            lambda r: r.exposure is Exposure.FIELD and r.plant.origin in UPLOAD_DIRS,
            code=upload,
            batch="region",
            refusals=HELD,
            not_built=SCRIPT_NOT_BUILT,
            other_refusal="ordinary upload refused",
            not_received="ordinary upload not received",
            ops=frozenset({"locator.setInputFiles"}),
            void=True,
        ),
        ChannelSpec(
            # The payload carries the agent's own value (buffer_code), so the Session
            # builds its run, as it does the typing run.
            ChannelId.UPLOAD_BUFFER,
            _A,
            lambda r: r.exposure is Exposure.FIELD
            and r.plant.kind is Kind.FILE
            and r.plant.origin is Origin.AGENT,
            refusals=HELD,
            not_built=SCRIPT_NOT_BUILT,
            ops=frozenset({"locator.setInputFiles"}),
            void=True,
        ),
        ChannelSpec(
            # Neo's own secret files (decision 33b): refused with DENY_CODE, naming no
            # path. The Session builds each run with the path its probe names
            # (deny_uploads); leak_judge.decide_neo_upload judges it with what the oracle
            # found the page read. Held today with the rest of setInputFiles.
            ChannelId.UPLOAD_NEO,
            _A,
            lambda r: r.exposure is Exposure.FIELD and r.plant.origin is Origin.NEO,
            refusals=HELD | {DENY_CODE},
            ops=frozenset({"locator.setInputFiles"}),
            void=True,
        ),
        ChannelSpec(
            # It answers nothing; what it puts in front is judged on the view displays
            # (FRONT_VIEW). Its fields are the page's, proven around it, which the view
            # cells stand on. Refused while the owner has control, like any write.
            ChannelId.BRING_TO_FRONT,
            _A,
            MAIN_FIELDS,
            code=bring_to_front,
            batch="page",
            refusals=frozenset({"host/owner_has_control"}),
            not_built=SCRIPT_NOT_BUILT,
            ops=frozenset({"page.bringToFront"}),
            void=True,
        ),
        ChannelSpec(
            ChannelId.SOLVE_CAPTCHA,
            _S,
            nothing,
            code=whole_page("neo.solveCaptcha(page)"),
            batch="page",
            not_built=frozenset({"host/captcha_unavailable"}),
        ),
        ChannelSpec(ChannelId.RUNS, Family.HOST_RECORD, everything),
        ChannelSpec(ChannelId.RUN, Family.HOST_RECORD, everything),
        ChannelSpec(ChannelId.OPERATION, Family.HOST_RECORD, everything),
        ChannelSpec(ChannelId.TRACE, Family.HOST_RECORD, everything),
        ChannelSpec(ChannelId.EVENTS, Family.HOST_RECORD, everything),
        ChannelSpec(ChannelId.EVENTS_REPLAY, Family.HOST_RECORD, everything),
        ChannelSpec(ChannelId.HOST_REPLIES, Family.HOST_REPLY, nothing),
        ChannelSpec(ChannelId.OP_SCREENSHOT, Family.LIVE_PIXELS, everything),
        ChannelSpec(ChannelId.SCREENCAST, Family.LIVE_PIXELS, everything),
        ChannelSpec(ChannelId.VIDEO, Family.LIVE_PIXELS, everything),
        ChannelSpec(ChannelId.DISPLAY_VIEW, Family.LIVE_PIXELS, everything),
        # A view display open while the agent brings a private tab forward, judged from
        # that moment rather than from when the tab went private (it was in the back).
        ChannelSpec(ChannelId.FRONT_VIEW, Family.LIVE_PIXELS, everything),
        ChannelSpec(ChannelId.CLIPBOARD_VIEW, Family.CLIPBOARD, everything),
        ChannelSpec(ChannelId.CLIPBOARD_CONTROL, Family.CLIPBOARD, everything),
        ChannelSpec(ChannelId.TRACE_STORE, Family.DISK, everything),
        ChannelSpec(ChannelId.STORE_SCREENSHOTS, Family.DISK, everything),
        ChannelSpec(ChannelId.CLAW_LOGS, Family.DISK, everything),
        ChannelSpec(ChannelId.DOCKER_LOGS, Family.DISK, everything),
        ChannelSpec(ChannelId.PROFILE, Family.INFO, everything),
    )
}
if set(SPECS) != set(ChannelId):  # a hand-synced list that cannot drift
    raise AssertionError(f"ChannelId without a spec: {set(ChannelId) - set(SPECS)}")

# Derived, never listed by hand: a new channel joins its scenarios through its flags.
ECHO_READS = tuple(c for c, s in SPECS.items() if s.reads_echoes)
FIELD_READS = tuple(c for c, s in SPECS.items() if s.reads_fields)
SCRIPT_READS = tuple(c for c, s in SPECS.items() if s.reads_script)
PIXEL_READS = tuple(c for c, s in SPECS.items() if s.family is Family.AGENT_PIXELS)
AFTER_RELEASE_READS = tuple(c for c, s in SPECS.items() if s.after_release)
KEY_READS: Mapping[Layout, tuple[ChannelId, ...]] = {
    layout: tuple(c for c, s in SPECS.items() if layout in s.keys)
    for layout in (Layout.FIELDS, Layout.ECHOES, Layout.LOCKED)
}
COPY_PROBES = tuple(c for c, s in SPECS.items() if s.family is Family.AGENT_KEYS)

# (act, channel): each pair gets a fresh FACTS instance, read cold then warm.
PAGE_ECHO_PROBES: tuple[tuple[Act, ChannelId], ...] = (
    (Act.ECHO_TITLE, ChannelId.TITLE),
    (Act.ECHO_TITLE, ChannelId.FACTS),
    (Act.ECHO_URL, ChannelId.FACTS),
    (Act.ALERT, ChannelId.DIALOG),
    (Act.DOWNLOAD, ChannelId.DOWNLOAD),
    (Act.POPUP, ChannelId.POPUP),
)

# The raw control display types only these per location; the takeover types every kind.
RACE_KINDS: frozenset[Kind] = frozenset({Kind.PASSWORD, Kind.CC_NUMBER, Kind.OTP})

# Ops the harness drives itself to open, wait for and close the fixture: not probes.
HARNESS_OPS = frozenset({"page.goto", "context.newPage", "page.close"})

# Every other op of browseros-playwright's table (ops.generated.ts), with why no probe
# reads page data through it. leak_sdk_test fails when the table gains an op that is in
# none of SPECS' ops, HARNESS_OPS or this map, so a new read cannot slip past the test.
UNPROBED_OPS: Mapping[str, str] = {
    **{
        op: "an action: returns no page text, and resolves through the one Subject "
        "(action/subject.rs) whose strict-mode preview fill and press are probed with"
        for op in (
            "locator.click",
            "locator.dblclick",
            "locator.hover",
            "locator.check",
            "locator.uncheck",
            "locator.dragTo",
            "locator.selectOption",
            "locator.selectText",
            "locator.blur",
            "locator.dispatchEvent",
        )
    },
    "locator.pressSequentially": "locator.press's runner (key_action.rs) typing text: "
    "press is probed, and the agent's typing is probed through keyboard.type",
    "keyboard.insertText": "keyboard.type's burst without key events (input_op.rs): "
    "the same focus walk marks a sensitive target, and it cannot copy",
    **{
        op: "the pointer: returns no page text, and copies nothing by itself"
        for op in ("mouse.move", "mouse.down", "mouse.up", "mouse.click", "mouse.wheel")
    },
    **{
        op: "returns no page text (booleans, counts, boxes, a wait)"
        for op in (
            "locator.scrollIntoViewIfNeeded",
            "locator.isEnabled",
            "locator.isDisabled",
            "locator.isEditable",
            "locator.isChecked",
            "locator.isVisible",
            "locator.isHidden",
            "locator.count",
            "locator.boundingBox",
            "locator.waitFor",
        )
    },
    "expect.page": "toHaveTitle and toHaveURL print the page facts that page.title and "
    "FACTS probe; the matcher message itself is expect.locator's, which is probed",
    "locator.elementHandles": "page.$$: the same handles as page.$, which is probed",
    "page.reload": "navigation: the page facts it records are probed by FACTS",
    "page.goBack": "navigation: the page facts it records are probed by FACTS",
    "page.goForward": "navigation: the page facts it records are probed by FACTS",
    "page.waitForURL": "navigation: the page facts it records are probed by FACTS",
    "page.opener": "returns a page handle, no page text",
    "events.listen": "page.on's subscription: the same events as events.wait",
    "events.next": "page.on's pump: the same events as events.wait",
    "dialog.accept": "the dialog event is probed with dismiss; same event",
    "download.saveAs": "writes a file the agent names; download.path is probed",
}


# Ops a probe exercises that ops.generated.ts does not list yet, with what serves them:
# the probe answers n/a (not built) until then, and leak_sdk_test fails on any probed op
# that is neither in the table nor here.
AWAITED_OPS: Mapping[str, str] = {
    "page.bringToFront": "sdk-playwright serves it (held-methods brief); until then the "
    "prelude refuses it (neo/unsupported)",
}


@dataclasses.dataclass(frozen=True)
class PlannedRun:
    location: Location | None  # None for page-level reads
    regions: tuple[Region, ...]  # what this run reads; the judge checks every region
    code: str


def plan_runs(
    spec: ChannelSpec,
    instance: Instance,
    *,
    exposures: Collection[Exposure] | None = None,
    kinds: Collection[Kind] | None = None,
    at: Location | None = None,
) -> tuple[PlannedRun, ...]:
    """The runs that exercise `spec` on the regions of `instance` it reaches (narrowed by
    `exposures` and `kinds`, and for a "location" batch to the run `at` one location), in
    the order the Session sends them. Regions no selector reaches (the closed shadow
    root) are asked for only by a probe that finds none by selector."""
    assert spec.code is not None, spec.id
    assert at is None or spec.batch == "location", spec.id
    targets = spec.targets or spec.reaches
    regions = [
        r
        for r in instance.regions
        if targets(r)
        and (PLACES[r.location].agent_reachable or not spec.by_selector)
        and (exposures is None or r.exposure in exposures)
        and (kinds is None or r.plant.kind in kinds)
    ]
    if spec.batch == "region":
        return tuple(
            PlannedRun(r.location, (r,), spec.code(instance, (r,), r.location))
            for r in regions
        )
    if spec.batch == "location":
        groups: dict[Location, list[Region]] = {}
        for r in regions:
            key = Location.MAIN if r.location is Location.SHADOW_OPEN else r.location
            groups.setdefault(key, []).append(r)
        order = [loc for loc in Location if loc in groups and at in (None, loc)]
        return tuple(
            PlannedRun(loc, tuple(groups[loc]), spec.code(instance, groups[loc], loc))
            for loc in order
        )
    if spec.batch == "exposure":
        echo = [r for r in regions if r.exposure in ECHO_LIKE]
        field = [r for r in regions if r.exposure not in ECHO_LIKE]
        return tuple(
            PlannedRun(None, tuple(g), spec.code(instance, g, None))
            for g in (echo, field)
            if g
        )
    if not regions and targets is not nothing:
        return ()
    return (PlannedRun(None, tuple(regions), spec.code(instance, regions, None)),)


def script_places(channel: ChannelId) -> tuple[Location | None, ...]:
    """Where a SCRIPT_READS probe's planned runs go, a fresh page each: one location per
    run of a "location" batch, None for one run over the page. Read off a SCRIPT page's
    plan, which every seed shares."""
    page = make_instance(Layout.SCRIPT, seed="places", index=0)
    return tuple(run.location for run in plan_runs(SPECS[channel], page))


class Contamination(AssertionError):
    """The harness was about to send a planted value or an anchor to Neo outside a
    designed injection. Every later grep would find the harness's own bytes."""


def assert_clean(
    request_text: str,
    instances: Iterable[Instance],
    *,
    inject: Plant | None = None,
    way: str | None = None,
) -> None:
    """Raise Contamination when `request_text` holds any plant's value of 4+ characters
    or any region's anchor, except `inject`'s own value, and only on the way its origin
    declares (DECLARED_WAYS_IN): a HOST value in POST /v1/secrets, an AGENT value in the
    run that types it. Every other request stays clean, the AGENT value's later reads
    included. A value is matched by its alphanumeric core, which every spelling (plain,
    JSON, URL, HTML, partly escaped) keeps."""
    if inject is not None and DECLARED_WAYS_IN.get(inject.origin) != way:
        raise Contamination(f"{inject.id} ({inject.origin}) has no way in by {way}")
    for instance in instances:
        for plant in instance.plants:
            core = plant.secret.reveal().removesuffix(SPECIAL_TAIL)
            if plant is not inject and len(core) >= 4 and core in request_text:
                raise Contamination(f"the value of {plant.id} in a request")
        for r in instance.regions:
            if r.anchor in request_text:
                raise Contamination(f"the anchor of {r.id} in a request")
