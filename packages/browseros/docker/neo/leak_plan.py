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
  request the harness sends (`Secret`, `assert_clean`).

Pure: no I/O, no clock, and no randomness beyond the run's seed.
"""

from __future__ import annotations

import dataclasses
import json
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
}
SENSITIVE_KINDS = tuple(k for k, s in KINDS.items() if s.sensitive)
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
    """Who put the value into the page. Neo never got any of them from the agent."""

    PAGE = "page"  # the page's script set it (prefill, the page's own input events)
    OWNER = "owner"  # typed on the control display
    HOST = "host"  # known only to the host, which registers it with POST /v1/secrets


class Secret:
    """One planted value. `str()`, `format()` and f-strings raise; only `reveal()` gives it.

    reveal() has five callers, each a designed way in or a reader of the record: the
    slices the oracle writes into the container, the owner's keystrokes, POST
    /v1/secrets, the evidence manifest, and the judge's needles. Everything else handles
    plants by id, and `assert_clean` backs this up at the wire.
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
    if layout is Layout.FIELDS:
        return [
            (kind, loc, page, False)
            for loc in FRAME_MATRIX
            for kind in (*SENSITIVE_KINDS, Kind.PLAIN)
        ] + [(kind, Location.MAIN, page, False) for kind in GAP_KINDS]
    if layout is Layout.ECHOES:
        return [(kind, Location.MAIN, page, False) for kind in Kind] + [
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
    if layout is Layout.ECHOES and plant.kind is not Kind.PLAIN:
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
    if layout is Layout.OWNER and plant.origin is Origin.OWNER:
        out = [(Exposure.ECHO, main)]
        if plant.navigates:
            out.append((Exposure.SERVER_ECHO, plant.location))
        if plant.kind is Kind.CC_NUMBER and plant.location is main:
            out.append((Exposure.PASTED, main))
        return out
    if layout is Layout.HOST:
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

    A registrable value adds one entry when swept; typed, the agent-path journal may add
    every 4+ character prefix too. The Session sums this as it plants and types and stops
    before REGISTRABLE_BUDGET, so the set's cap can never evict a value and fake a leak.
    """
    if not plant.registrable:
        return 0
    return len(plant.secret) - 2 if typed else 1


def neutral_tokens(instance: Instance) -> list[str]:
    """Every token the page renders that is meant to classify as nothing."""
    tokens = [instance.name, "ECHO", "Field", "Box"]
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
        }[phase]
        if not ok:
            raise PhaseError(f"{name}: {phase} does not hold ({runs} runs)")
        self._journal_pending.discard(name)

    def note_touched(self, instance: Instance) -> None:
        self._touched.add(instance.name)

    def note_registered(self, instance: Instance) -> None:
        self._touched.add(instance.name)
        self._registered.add(instance.name)
        self._journal_pending.discard(instance.name)

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
    TO_HAVE_VALUE = "expect.toHaveValue"
    FACTS = "page facts"
    DIALOG = "dialog event"
    DOWNLOAD = "download event"
    POPUP = "popup event"
    REPLAY = "requestId replay"
    SCREENSHOT = "page.screenshot"
    PDF = "page.pdf"
    EVALUATE = "page.evaluate"
    FRAME_EVALUATE = "frame.evaluate"
    WAIT_FOR_FUNCTION = "page.waitForFunction"
    LOCATOR_EVALUATE = "locator.evaluate"
    EVALUATE_ALL = "locator.evaluateAll"
    EVAL_ONE = "page.$eval"
    EVAL_ALL = "page.$$eval"
    HANDLE_EVALUATE = "elementHandle.evaluate"
    SET_CONTENT = "page.setContent"
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
    "exposure" (fields and echoes apart), "page" (one run), or "none" (not an agent run).
    """

    id: ChannelId
    family: Family
    reaches: Reaches
    code: CodeTemplate | None = None
    batch: str = "none"
    refusals: frozenset[str] = frozenset()  # "owner/code" answers that mean withheld
    not_built: frozenset[str] = frozenset()  # answers that mean the method is not built
    ops: frozenset[str] = frozenset()  # ops.generated.ts ops this probe exercises
    reads_fields: bool = False  # in FIELD_READS (scenario "fields")
    reads_echoes: bool = (
        False  # in ECHO_READS (scenario "echoes": a cold instance each)
    )
    after_release: bool = False  # in AFTER_RELEASE_READS
    l2_only: bool = False  # the design masks this exit with L2 only (residual BD-L2)
    # Some answers show only some of the regions they reach (a strict-mode error
    # previews ten elements): an unlisted region is n/a there. `answer_errors` are
    # errors whose message is the probe's answer rather than a refusal.
    lists_some: bool = False
    answer_errors: frozenset[str] = frozenset()


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


def keyed(read_for: Callable[[Region], str]) -> CodeTemplate:
    """One run per location, one try per region; `read_for(region)` is the JS applied to
    the region's locator `l` (and its `id`), e.g. "l.inputValue({timeout: 2000})"."""

    def template(
        instance: Instance, regions: Sequence[Region], location: Location | None
    ) -> str:
        lines = [f"const root = {_root(instance, location)};", _KEYED_HEAD]
        lines += [
            f"await read({json.dumps(r.id)}, (l, id) => {read_for(r)});"
            for r in regions
        ]
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


def main_world(call: str) -> CodeTemplate:
    """Main-world script asking for raw values: it must come back held. `call` uses IDS
    (the region ids as a JSON array: ids only, never values) and ID (the first one)."""

    def template(
        instance: Instance, regions: Sequence[Region], location: Location | None
    ) -> str:
        ids = [r.id for r in regions]
        return (
            f"const IDS = {json.dumps(ids)};\nconst ID = IDS[0];\nreturn await {call};"
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
            ChannelId.TO_HAVE_VALUE,
            _A,
            reach(fields=AGENT_FIELDS),
            code=keyed(same(f"expect(l).toHaveValue('x', {_TIMEOUT})")),
            batch="location",
            not_built=NOT_BUILT,
            ops=frozenset({"expect.locator"}),
            reads_fields=True,
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
        ChannelSpec(
            ChannelId.EVALUATE,
            _S,
            MAIN_FIELDS,
            code=main_world(
                "page.evaluate((ids) => ids.map((id) => document.getElementById(id).value), IDS)"
            ),
            batch="page",
            refusals=HELD,
            not_built=NOT_BUILT,
            ops=frozenset({"page.evaluate"}),
        ),
        ChannelSpec(
            ChannelId.FRAME_EVALUATE,
            _S,
            MAIN_FIELDS,
            code=main_world(
                "page.frames()[0].evaluate((ids) => ids.map((id) => document.getElementById(id).value), IDS)"
            ),
            batch="page",
            refusals=HELD,
            not_built=NOT_BUILT,
            ops=frozenset({"page.evaluate", "page.frames"}),
        ),
        ChannelSpec(
            ChannelId.WAIT_FOR_FUNCTION,
            _S,
            MAIN_FIELDS,
            code=main_world(
                "page.waitForFunction((id) => document.getElementById(id).value.length > 0, ID, {timeout: 2000})"
            ),
            batch="page",
            refusals=HELD,
            not_built=NOT_BUILT,
            ops=frozenset({"page.waitForFunction"}),
        ),
        ChannelSpec(
            ChannelId.LOCATOR_EVALUATE,
            _S,
            MAIN_FIELDS,
            code=main_world("page.locator('#' + ID).evaluate((el) => el.value)"),
            batch="page",
            refusals=HELD,
            not_built=NOT_BUILT,
            ops=frozenset({"locator.evaluate"}),
        ),
        ChannelSpec(
            ChannelId.EVALUATE_ALL,
            _S,
            MAIN_FIELDS,
            code=main_world(
                "page.locator('input').evaluateAll((els) => els.map((el) => el.value))"
            ),
            batch="page",
            refusals=HELD,
            not_built=NOT_BUILT,
            ops=frozenset({"locator.evaluateAll"}),
        ),
        ChannelSpec(
            ChannelId.EVAL_ONE,
            _S,
            MAIN_FIELDS,
            code=main_world("page.$eval('#' + ID, (el) => el.value)"),
            batch="page",
            refusals=HELD,
            not_built=NOT_BUILT,
            ops=frozenset({"locator.evaluate"}),
        ),
        ChannelSpec(
            ChannelId.EVAL_ALL,
            _S,
            MAIN_FIELDS,
            code=main_world("page.$$eval('input', (els) => els.map((el) => el.value))"),
            batch="page",
            refusals=HELD,
            not_built=NOT_BUILT,
            ops=frozenset({"locator.evaluateAll"}),
        ),
        ChannelSpec(
            ChannelId.HANDLE_EVALUATE,
            _S,
            MAIN_FIELDS,
            code=main_world(
                "page.$('#' + ID).then((h) => h.evaluate((el) => el.value))"
            ),
            batch="page",
            refusals=HELD,
            not_built=NOT_BUILT,
            ops=frozenset({"locator.elementHandle", "locator.evaluate"}),
        ),
        ChannelSpec(
            ChannelId.SET_CONTENT,
            _S,
            MAIN_FIELDS,
            code=main_world(
                "page.setContent('<p>neutral</p><script>document.title = document.body.textContent</script>')"
            ),
            batch="page",
            refusals=HELD,
            ops=frozenset({"page.setContent"}),
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
SCRIPT_READS = tuple(c for c, s in SPECS.items() if s.family is Family.AGENT_SCRIPT)
PIXEL_READS = tuple(c for c, s in SPECS.items() if s.family is Family.AGENT_PIXELS)
AFTER_RELEASE_READS = tuple(c for c, s in SPECS.items() if s.after_release)

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
        op: "an action: not built (neo/internal_error), and the agent never types"
        for op in (
            "locator.click",
            "locator.dblclick",
            "locator.hover",
            "locator.check",
            "locator.uncheck",
            "locator.dragTo",
            "locator.fill",
            "locator.press",
            "locator.pressSequentially",
            "locator.selectOption",
            "locator.selectText",
            "locator.focus",
            "locator.blur",
            "locator.setInputFiles",
            "locator.dispatchEvent",
            "keyboard.down",
            "keyboard.up",
            "keyboard.press",
            "keyboard.type",
            "keyboard.insertText",
            "mouse.move",
            "mouse.down",
            "mouse.up",
            "mouse.click",
            "mouse.wheel",
        )
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
    "expect.page": "not built (neo/internal_error); expect.locator is probed",
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
) -> tuple[PlannedRun, ...]:
    """The runs that exercise `spec` on the regions of `instance` it reaches (narrowed by
    `exposures` and `kinds`), in the order the Session sends them. Regions no selector
    reaches (the closed shadow root) are never asked for."""
    assert spec.code is not None, spec.id
    regions = [
        r
        for r in instance.regions
        if spec.reaches(r)
        and PLACES[r.location].agent_reachable
        and (exposures is None or r.exposure in exposures)
        and (kinds is None or r.plant.kind in kinds)
    ]
    if spec.batch == "location":
        groups: dict[Location, list[Region]] = {}
        for r in regions:
            key = Location.MAIN if r.location is Location.SHADOW_OPEN else r.location
            groups.setdefault(key, []).append(r)
        order = [loc for loc in Location if loc in groups]
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
    if not regions and spec.reaches is not nothing:
        return ()
    return (PlannedRun(None, tuple(regions), spec.code(instance, regions, None)),)


class Contamination(AssertionError):
    """The harness was about to send a planted value or an anchor to Neo outside a
    designed injection. Every later grep would find the harness's own bytes."""


def assert_clean(
    request_text: str, instances: Iterable[Instance], *, inject: Plant | None = None
) -> None:
    """Raise Contamination when `request_text` holds any plant's value of 4+ characters
    or any region's anchor, except `inject`'s own value: owner keys and POST /v1/secrets
    are the designed ways in. A value is matched by its alphanumeric core, which every
    spelling (plain, JSON, URL, HTML, partly escaped) keeps."""
    for instance in instances:
        for plant in instance.plants:
            core = plant.secret.reveal().removesuffix(SPECIAL_TAIL)
            if plant is not inject and len(core) >= 4 and core in request_text:
                raise Contamination(f"the value of {plant.id} in a request")
        for r in instance.regions:
            if r.anchor in request_text:
                raise Contamination(f"the anchor of {r.id} in a request")
