"""Unit tests for the leak test's pure halves, leak_plan.py and leak_judge.py. No Docker.

The end-to-end run is the test of leak_sdk.py itself. These pin the parts whose mistakes
would make its table lie: values that collide or classify by accident, a detector that
misses a spelling or matches a timestamp, a verdict without proof, a residual that
swallows a new leak, a report that prints a value. Two tests read the repo's own sources
(rules.json and ops.generated.ts), so a change there fails here first.

  python3 -m unittest leak_sdk_test
"""

from __future__ import annotations

import dataclasses
import html
import json
import posixpath
import re
import shutil
import struct
import tempfile
import unittest
import urllib.parse
from pathlib import Path

import leak_judge as J
import leak_plan as P
import leak_sdk as S
from leak_wire_test import (  # noqa: F401 - one entry point runs the wire's tests too
    FixtureHostsTest,
    MjpegParserTest,
    Mp4BoxesTest,
    RfbParserTest,
    RunReplyTest,
    SseParserTest,
)

AGENT = Path(__file__).resolve().parents[3] / "browseros-agent"
RULES = AGENT / "crates/browseros-core/src/sensitive_fields/rules.json"
OPS = AGENT / "packages/browseros-playwright-js/src/prelude/ops.generated.ts"


def run_instances(n: int = 8) -> list[P.Instance]:
    """n instances of every layout, as one run's: indices unique and two digits, as a
    region id holds its instance's index in two (r<index><counter>)."""
    assert n * len(P.Layout) <= 100
    return [
        P.make_instance(layout, seed="unit", index=i * len(P.Layout) + j)
        for i in range(n)
        for j, layout in enumerate(P.Layout)
    ]


def echo_of(instance: P.Instance, plant: P.Plant) -> P.Region:
    return next(r for r in instance.regions_of(plant) if r.exposure is P.Exposure.ECHO)


def obs(region: P.Region, **kw) -> J.Observation:
    base = dict(
        channel=P.ChannelId.TEXT_CONTENT,
        region=region,
        phase=P.Phase.COLD,
        source="c1",
        covered=True,
        reachable=True,
        present=True,
    )
    return J.Observation(**{**base, **kw})


def cell(
    o: J.Observation, verdict: J.Verdict | None = None, run: str | None = None
) -> J.Cell:
    v, why = J.decide(o)
    return J.Cell(
        f"{o.source}:{o.region.id}",
        o.channel,
        o.region,
        o.phase,
        verdict or v,
        why,
        o.source,
        run,
        o.hit,
    )


class InstanceTests(unittest.TestCase):
    def test_values_are_unique_across_every_plant_of_a_run(self) -> None:
        instances = run_instances()
        J.Detector(instances)  # raises when two plants share an 8-gram
        values = [
            p.secret.reveal() for i in instances for p in i.plants if len(p.secret) > 6
        ]
        self.assertEqual(len(values), len(set(values)))
        ids = [r.id for i in instances for r in i.regions]
        anchors = [r.anchor for i in instances for r in i.regions]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertEqual(len(anchors), len(set(anchors)))

    def test_value_shapes(self) -> None:
        instance = P.make_instance(P.Layout.FIELDS, seed="shapes", index=1)
        for plant in instance.plants:
            value = plant.secret.reveal()
            shape = P.KINDS[plant.kind].shape
            if plant.kind is P.Kind.OTP_SPLIT:
                self.assertRegex(value, r"^\d{6}$")
                self.assertEqual(len(instance.field_regions(plant)), 6)
            elif shape is P.Shape.CARD:
                self.assertRegex(value, r"^\d{16}$")
                self.assertTrue(P.luhn_ok(value))
            elif shape is P.Shape.CVC:
                self.assertRegex(value, r"^\d{3}$")
            else:
                self.assertRegex(value, r"^[A-Za-z][A-Za-z0-9]{11}(&<\")?$")
        special = [
            p for p in instance.plants if p.secret.reveal().endswith(P.SPECIAL_TAIL)
        ]
        self.assertEqual(
            [(p.kind, p.location) for p in special],
            [(P.Kind.PASSWORD, P.Location.MAIN)],
        )

    def test_neutral_tokens_never_match_a_rule_word(self) -> None:
        rules = json.loads(RULES.read_text())
        for instance in run_instances(2):
            with self.subTest(layout=instance.layout):
                self.assertEqual(P.lint_instance(instance, rules), [])

    def test_lint_catches_a_label_that_classifies_by_the_wrong_rule(self) -> None:
        rules = json.loads(RULES.read_text())
        rules["words"].append({"kind": "password", "pattern": "field"})
        problems = P.lint_instance(
            P.make_instance(P.Layout.FIELDS, seed="x", index=1), rules
        )
        self.assertTrue(any("Field" in p for p in problems))

    def test_same_seed_and_index_give_the_same_instance(self) -> None:
        a = P.make_instance(P.Layout.OWNER, seed="s", index=4)
        b = P.make_instance(P.Layout.OWNER, seed="s", index=4)
        self.assertEqual(J.instance_to_json(a), J.instance_to_json(b))
        self.assertEqual(
            J.instance_to_json(J.instance_from_json(J.instance_to_json(a))),
            J.instance_to_json(a),
        )

    def test_registrable_counts_a_typed_value_twice_and_skips_short_values(
        self,
    ) -> None:
        """A typed value is registered whole once quiet, plus at most the prefix a read
        swept between two keystrokes; a value under four characters or a split code adds
        nothing."""
        instance = P.make_instance(P.Layout.OWNER, seed="r", index=1)
        cvc = instance.plant(P.Kind.CC_CSC, P.Location.MAIN, origin=P.Origin.OWNER)
        split = instance.plant(P.Kind.OTP_SPLIT, P.Location.MAIN, origin=P.Origin.OWNER)
        pw = instance.plant(
            P.Kind.WORD_PASSWORD, P.Location.MAIN, origin=P.Origin.OWNER
        )
        plain = instance.plant(P.Kind.PLAIN, P.Location.MAIN)
        self.assertEqual(P.registrable(cvc, typed=True), 0)
        self.assertEqual(P.registrable(split, typed=True), 0)
        self.assertEqual(P.registrable(plain, typed=False), 0)
        self.assertEqual(P.registrable(pw, typed=False), 1)
        self.assertEqual(P.registrable(pw, typed=True), 2)

    def test_the_typed_layout_types_the_closed_root_first_and_echoes_each(self) -> None:
        """The seal cannot hear input in the closed root, so its plants are typed first
        and a journaled field's typing comes last; every value the agent types is empty
        in the page until then and echoed in MAIN once typed."""
        instance = P.make_instance(P.Layout.TYPED, seed="typed", index=1)
        self.assertTrue(all(p.origin is P.Origin.AGENT for p in instance.plants))
        locations = [p.location for p in instance.plants[1:]]
        closed = locations.count(P.Location.SHADOW_CLOSED)
        self.assertEqual(locations[:closed], [P.Location.SHADOW_CLOSED] * closed)
        self.assertTrue(P.KINDS[instance.plants[-1].kind].sensitive)
        self.assertNotEqual(instance.plants[-1].location, P.Location.SHADOW_CLOSED)
        for plant in instance.plants:
            (echo,) = [
                r for r in instance.regions_of(plant) if r.exposure is P.Exposure.ECHO
            ]
            self.assertIs(echo.location, P.Location.MAIN)
        main = instance.slice(P.Location.MAIN)["regions"]
        self.assertFalse(any("value" in r for r in main))

    def test_every_region_is_in_exactly_one_document(self) -> None:
        for layout in P.Layout:
            instance = P.make_instance(layout, seed="slice", index=2)
            seen = [
                r["id"]
                for d in instance.documents()
                for r in instance.slice(d)["regions"]
            ]
            seen += [e["region"]["id"] for e in instance.server_echoes().values()]
            with self.subTest(layout=layout):
                self.assertEqual(sorted(seen), sorted(r.id for r in instance.regions))

    def test_a_slice_carries_values_only_for_what_the_page_plants(self) -> None:
        instance = P.make_instance(P.Layout.OWNER, seed="v", index=3)
        main = instance.slice(P.Location.MAIN)
        for region in main["regions"]:
            plant = next(p for p in instance.plants if p.id == region["plant"])
            with self.subTest(region=region["id"]):
                self.assertEqual(
                    "value" in region,
                    plant.origin is P.Origin.PAGE and region["exposure"] == "field",
                )
        self.assertIn("shadow_closed", {r["location"] for r in main["regions"]})
        frames = {f["id"]: f["late"] for f in main["frames"]}
        self.assertEqual(
            frames,
            {
                "fr-so": False,
                "fr-ss": False,
                "fr-xo": False,
                "fr-nav": False,
                "fr-late": True,
            },
        )
        self.assertEqual(
            [f["id"] for f in instance.slice(P.Location.CROSS_SITE)["frames"]],
            ["fr-xn"],
        )


class SecretTests(unittest.TestCase):
    def test_a_secret_refuses_str_format_and_fstrings(self) -> None:
        secret = P.Secret("Abcdefgh2345")
        with self.assertRaises(TypeError):
            str(secret)
        with self.assertRaises(TypeError):
            f"{secret}"  # noqa: B018
        with self.assertRaises(TypeError):
            "{}".format(secret)
        self.assertNotIn("Abcd", repr(secret))

    def test_assert_clean_stops_values_and_anchors_in_any_spelling(self) -> None:
        instance = P.make_instance(P.Layout.FACTS, seed="clean", index=1)
        special = instance.plant(P.Kind.PASSWORD, P.Location.MAIN)
        value = special.secret.reveal()
        for text in (
            value,
            json.dumps(value),
            re.sub("&", "%26", value),
            instance.regions[0].anchor,
        ):
            with self.subTest(text=text[:4]), self.assertRaises(P.Contamination):
                P.assert_clean(f"code {text} end", [instance])
        P.assert_clean(f"secret {value}", [instance], inject=special)
        other = instance.plant(P.Kind.OTP)
        with self.assertRaises(P.Contamination):
            P.assert_clean(
                f"{value} {other.secret.reveal()}", [instance], inject=special
            )

    def test_an_agent_value_rides_only_its_typing_run(self) -> None:
        """The agent's typing is a declared way in: its run's code carries its one value,
        and nothing else carries it, nor any other value. A HOST value is declared for
        POST /v1/secrets only, so it may not ride a run either."""
        instance = P.make_instance(P.Layout.TYPED, seed="ways", index=1)
        plant = instance.plant(P.Kind.PASSWORD, P.Location.MAIN)
        (field,) = instance.field_regions(plant)
        code = P.typing_code(instance, field)
        run = P.DECLARED_WAYS_IN[P.Origin.AGENT]
        P.assert_clean(code, [instance], inject=plant, way=run)
        self.assertNotIn(field.anchor, code)
        for kw in ({}, {"inject": plant, "way": P.DECLARED_WAYS_IN[P.Origin.HOST]}):
            with self.subTest(kw=kw), self.assertRaises(P.Contamination):
                P.assert_clean(code, [instance], **kw)
        other = instance.plant(P.Kind.OTP, P.Location.SAME_SITE)
        with self.assertRaises(P.Contamination):
            P.assert_clean(
                f"{code} {other.secret.reveal()}", [instance], inject=plant, way=run
            )
        host = P.make_instance(P.Layout.HOST, seed="ways", index=2)
        secret = host.plant(origin=P.Origin.HOST)
        with self.assertRaises(P.Contamination):
            P.assert_clean(secret.secret.reveal(), [host], inject=secret, way=run)

    def test_no_run_template_ever_holds_a_value_or_an_anchor(self) -> None:
        for layout in P.Layout:
            instance = P.make_instance(layout, seed="tpl", index=5)
            for channel, spec in P.SPECS.items():
                if spec.code is None:
                    continue
                for run in P.plan_runs(spec, instance):
                    with self.subTest(layout=layout, channel=channel, at=run.location):
                        P.assert_clean(run.code, [instance])


class PhaseGuardTests(unittest.TestCase):
    def setUp(self) -> None:
        self.guard = P.PhaseGuard()
        self.inst = P.make_instance(P.Layout.ECHOES, seed="g", index=1)

    def test_cold_only_as_the_first_touch(self) -> None:
        self.guard.claim(self.inst, P.Phase.COLD, 1)
        self.guard.note_touched(self.inst)
        with self.assertRaises(P.PhaseError):
            self.guard.claim(self.inst, P.Phase.COLD, 1)

    def test_cold_refuses_a_read_that_needs_two_runs(self) -> None:
        with self.assertRaises(P.PhaseError):
            self.guard.claim(self.inst, P.Phase.COLD, 2)

    def test_warm_needs_a_registration(self) -> None:
        with self.assertRaises(P.PhaseError):
            self.guard.claim(self.inst, P.Phase.WARM, 1)
        self.guard.note_registered(self.inst)
        self.guard.claim(self.inst, P.Phase.WARM, 3)

    def test_journaled_needs_a_touch_first_and_holds_for_one_read(self) -> None:
        with self.assertRaises(P.PhaseError):
            self.guard.note_late_plant(self.inst)
        self.guard.note_touched(self.inst)
        self.guard.note_late_plant(self.inst)
        self.guard.claim(self.inst, P.Phase.JOURNALED, 1)
        with self.assertRaises(P.PhaseError):
            self.guard.claim(self.inst, P.Phase.JOURNALED, 1)

    def test_agent_typed_holds_from_the_typing_until_a_registration(self) -> None:
        with self.assertRaises(P.PhaseError):
            self.guard.claim(self.inst, P.Phase.AGENT_TYPED, 1)
        self.guard.note_typed(self.inst)
        self.guard.claim(self.inst, P.Phase.AGENT_TYPED, 2)
        with self.assertRaises(P.PhaseError):
            self.guard.claim(self.inst, P.Phase.COLD, 1)
        self.guard.note_registered(self.inst)
        with self.assertRaises(P.PhaseError):
            self.guard.claim(self.inst, P.Phase.AGENT_TYPED, 1)
        self.guard.claim(self.inst, P.Phase.WARM, 1)

    def test_after_content_holds_once_the_agent_wrote_the_page(self) -> None:
        with self.assertRaises(P.PhaseError):
            self.guard.claim(self.inst, P.Phase.AFTER_CONTENT, 1)
        self.guard.note_written(self.inst)
        self.guard.claim(self.inst, P.Phase.AFTER_CONTENT, 3)
        with self.assertRaises(P.PhaseError):
            self.guard.claim(self.inst, P.Phase.COLD, 1)

    def test_control_phases_follow_the_open_control(self) -> None:
        with self.assertRaises(P.PhaseError):
            self.guard.claim(self.inst, P.Phase.DURING_CONTROL, 1)
        self.guard.note_control("takeover", self.inst)
        self.guard.claim(self.inst, P.Phase.DURING_CONTROL, 1)
        with self.assertRaises(P.PhaseError):
            self.guard.claim(self.inst, P.Phase.CONTROL_RACE, 1)
        self.guard.note_control(None)
        self.guard.claim(self.inst, P.Phase.AFTER_RELEASE, 4)


class ChannelTableTests(unittest.TestCase):
    def test_every_channel_id_has_exactly_one_spec(self) -> None:
        self.assertEqual(set(P.SPECS), set(P.ChannelId))
        for key, spec in P.SPECS.items():
            self.assertIs(spec.id, key)

    def test_derived_scenario_lists_follow_the_flags(self) -> None:
        self.assertIn(P.ChannelId.STRICT_PREVIEW, P.ECHO_READS)
        self.assertNotIn(P.ChannelId.INPUT_VALUE, P.ECHO_READS)
        self.assertEqual(set(P.PIXEL_READS), {P.ChannelId.SCREENSHOT, P.ChannelId.PDF})
        self.assertTrue(all(P.SPECS[c].reads_fields for c in P.FIELD_READS))
        self.assertEqual(
            set(P.SCRIPT_READS),
            {
                P.ChannelId.EVALUATE,
                P.ChannelId.FRAME_EVALUATE,
                P.ChannelId.WAIT_FOR_FUNCTION,
                P.ChannelId.LOCATOR_EVALUATE,
                P.ChannelId.EVALUATE_ALL,
                P.ChannelId.EVAL_ONE,
                P.ChannelId.EVAL_ALL,
                P.ChannelId.HANDLE_EVALUATE,
            },
            "every served main-world read but the opener's, which needs the popup",
        )
        self.assertNotIn(P.ChannelId.TO_HAVE_VALUE, P.FIELD_READS)
        self.assertIn(P.ChannelId.TO_HAVE_TEXT, P.KEY_READS[P.Layout.ECHOES])
        self.assertIn(P.ChannelId.FILL_CALL_LOG, P.KEY_READS[P.Layout.LOCKED])

    def test_copy_probes_cut_last(self) -> None:
        """A cut that goes through empties its field, so every copy chord runs before
        any cut on the same field."""
        cuts = [
            n for n, c in enumerate(P.COPY_PROBES) if "Control+X" in c or "Delete" in c
        ]
        copies = [n for n, c in enumerate(P.COPY_PROBES) if n not in cuts]
        self.assertTrue(cuts and copies)
        self.assertLess(max(copies), min(cuts))

    def test_every_playwright_op_is_probed_driven_or_excused(self) -> None:
        names = set(re.findall(r"^  '([^']+)': \{", OPS.read_text(), re.MULTILINE))
        self.assertTrue(names, OPS)
        probed = {op for spec in P.SPECS.values() for op in spec.ops}
        covered = probed | P.HARNESS_OPS | set(P.UNPROBED_OPS)
        self.assertEqual(
            sorted(names - covered), [], "ops neither probed, driven nor excused"
        )
        self.assertEqual(
            sorted(set(P.UNPROBED_OPS) & probed), [], "an excused op is probed"
        )
        self.assertEqual(
            sorted(probed - names - set(P.AWAITED_OPS)),
            [],
            "a probe names an op the crate lacks and nobody awaits",
        )
        self.assertNotIn("locator.setInputFiles", P.UNPROBED_OPS)
        self.assertIn("page.bringToFront", probed)


def want_of(code: str) -> object:
    """The WANT a main-world probe's run code hands its reader: ids and paths only."""
    found = re.search(r"^const WANT = (.*);$", code, re.MULTILINE)
    assert found, code
    return json.loads(found[1])


class ScriptPlanTests(unittest.TestCase):
    """The main-world probes' plan: where each runs, what it asks, in which spellings."""

    def setUp(self) -> None:
        self.inst = P.make_instance(P.Layout.SCRIPT, seed="sp", index=1)

    def test_each_script_probe_is_one_run_on_a_page_of_its_own(self) -> None:
        """A script read is cold only as the first touch of fresh values, in one run, so
        each place a probe runs gets a page of its own; between them the places cover
        every region the probe reaches, each once."""
        for channel in P.SCRIPT_READS:
            spec = P.SPECS[channel]
            asked: list[str] = []
            for at in P.script_places(channel):
                runs = P.plan_runs(spec, self.inst, at=at)
                with self.subTest(channel=channel, at=at):
                    self.assertEqual(len(runs), 1)
                asked += [r.id for r in runs[0].regions]
            reached = [r.id for r in self.inst.regions if spec.reaches(r)]
            with self.subTest(channel=channel):
                self.assertEqual(sorted(asked), sorted(reached))

    def test_long_values_are_asked_in_three_spellings_and_short_ones_whole(
        self,
    ) -> None:
        """D3's guard masks a value of 4+ characters plain, JSON-escaped and
        percent-encoded, and a shorter one only as a whole string: a probe asks for
        exactly those, so a raw answer is a leak and never deliberate encoding."""
        for channel in (P.ChannelId.EVALUATE, P.ChannelId.EVALUATE_ALL):
            spec = P.SPECS[channel]
            for run in P.plan_runs(spec, self.inst):
                want = want_of(run.code)
                pairs = (
                    want.items()
                    if isinstance(want, dict)
                    else [(entry[0], entry[2]) for entry in want]
                )
                for region_id, spellings in pairs:
                    short = self.inst.region(region_id).short
                    with self.subTest(channel=channel, region=region_id):
                        expected = ["plain"] if short else ["plain", "json", "uri"]
                        self.assertEqual(spellings, expected)

    def test_the_top_world_reaches_what_the_pages_own_script_can(self) -> None:
        """page.evaluate runs in the top document: its fields, both shadow roots (the
        closed one through the page's own reference), the same-origin frame, the a.test
        frame inside b.test, and the echoes. Cross-origin frames stay out; a selector
        probe never asks the closed root."""
        (run,) = P.plan_runs(P.SPECS[P.ChannelId.EVALUATE], self.inst)
        steps = {entry[0]: entry[1] for entry in want_of(run.code)}
        places = {self.inst.region(i).location for i in steps}
        self.assertEqual(
            places,
            {
                P.Location.MAIN,
                P.Location.SHADOW_OPEN,
                P.Location.SHADOW_CLOSED,
                P.Location.SAME_ORIGIN,
                P.Location.NESTED,
            },
        )
        for region_id, path in steps.items():
            location = self.inst.region(region_id).location
            with self.subTest(region=region_id):
                self.assertEqual(
                    path, json.loads(json.dumps(P.MAIN_WORLD_STEPS[location]))
                )
        for location in P.MAIN_WORLD_STEPS:
            self.assertEqual(P.PLACES[location].host, P.PLACES[P.Location.MAIN].host)
        for run in P.plan_runs(P.SPECS[P.ChannelId.LOCATOR_EVALUATE], self.inst):
            self.assertNotIn(
                P.Location.SHADOW_CLOSED, {r.location for r in run.regions}
            )

    def test_the_opener_probe_reads_the_page_from_its_popup(self) -> None:
        (run,) = P.plan_runs(P.SPECS[P.ChannelId.OPENER_EVALUATE], self.inst)
        self.assertIn("context.pages().find(", run.code)
        for entry in want_of(run.code):
            with self.subTest(region=entry[0]):
                self.assertEqual(entry[1][0], ["opener", None])
        self.assertNotIn(P.Location.SHADOW_CLOSED, {r.location for r in run.regions})

    def test_a_script_page_serves_a_popup_with_nothing_of_its_own(self) -> None:
        self.assertIn(P.Location.POPUP, self.inst.documents())
        self.assertEqual(self.inst.slice(P.Location.POPUP)["regions"], [])
        self.assertEqual(
            [p.kind for p in self.inst.plants],
            [
                p.kind
                for p in P.make_instance(P.Layout.ECHOES, seed="sp", index=2).plants
            ],
        )


class UploadPlanTests(unittest.TestCase):
    """setInputFiles' ordinary probes: files on Neo's disk by path (one in the downloads
    dir) and the agent's own buffer."""

    def setUp(self) -> None:
        self.inst = P.make_instance(P.Layout.UPLOAD, seed="up", index=1)
        self.disk = self.inst.plant(origin=P.Origin.FILE)
        self.download = self.inst.plant(origin=P.Origin.DOWNLOAD)
        self.buffer = self.inst.plant(origin=P.Origin.AGENT)

    def test_an_upload_page_plants_files_on_neos_disk_and_an_agent_buffer(
        self,
    ) -> None:
        """Each goes into a file input no rule classifies and is echoed in the top
        document. None is in the page's slice: two sit in files, the other in the
        agent's run."""
        for plant in (self.disk, self.download, self.buffer):
            self.assertIs(plant.kind, P.Kind.FILE)
            (field,) = self.inst.field_regions(plant)
            self.assertEqual(dict(P.KINDS[plant.kind].attrs), {"type": "file"})
            self.assertEqual(
                [r.exposure for r in self.inst.regions_of(plant)],
                [P.Exposure.FIELD, P.Exposure.ECHO],
            )
            self.assertEqual(P.registrable(plant, typed=True), 0)
        main = self.inst.slice(P.Location.MAIN)["regions"]
        self.assertFalse(any("value" in r for r in main))
        self.assertNotIn(P.Kind.FILE, P.SENSITIVE_KINDS)
        for layout in (P.Layout.ECHOES, P.Layout.SCRIPT, P.Layout.FIELDS):
            kinds = {p.kind for p in P.make_instance(layout, seed="up", index=2).plants}
            self.assertNotIn(P.Kind.FILE, kinds)

    def test_each_path_upload_names_an_ordinary_file_and_no_value(self) -> None:
        """One run per file, each naming the path the oracle writes it at: one outside
        Neo's state, one in Neo's downloads dir (agents re-upload what they downloaded),
        neither under a directory the deny list covers (decision 33b)."""
        runs = P.plan_runs(P.SPECS[P.ChannelId.UPLOAD], self.inst)
        (disk,) = self.inst.field_regions(self.disk)
        (download,) = self.inst.field_regions(self.download)
        self.assertEqual([run.regions for run in runs], [(disk,), (download,)])
        paths = [P.upload_path(field) for field in (disk, download)]
        self.assertTrue(paths[0].startswith("/tmp/"))
        self.assertEqual(paths[1], f"/data/downloads/{download.id}.txt")
        for run, path in zip(runs, paths, strict=True):
            self.assertIn(f"setInputFiles({json.dumps(path)})", run.code)
            for denied in ("/data/browserclaw", "/data/profile", "/run/"):
                self.assertFalse(path.startswith(denied), path)
            P.assert_clean(run.code, [self.inst])

    def test_the_buffer_upload_is_the_one_run_that_carries_its_own_value(self) -> None:
        """A {name, mimeType, buffer} payload is the agent's value in its own code: a
        declared way in, as typing is, and no other request may carry it."""
        (field,) = self.inst.field_regions(self.buffer)
        code = P.buffer_code(self.inst, field)
        way = P.DECLARED_WAYS_IN[P.Origin.AGENT]
        P.assert_clean(code, [self.inst], inject=self.buffer, way=way)
        self.assertIn(".setInputFiles({name: 'upload.txt'", code)
        with self.assertRaises(P.Contamination):
            P.assert_clean(code, [self.inst])
        self.assertIsNone(P.SPECS[P.ChannelId.UPLOAD_BUFFER].code)


class DenyPlanTests(unittest.TestCase):
    """setInputFiles' deny list (decision 33b): each of Neo's own secret files by its
    path, and the shapes that hide a path, each probe on a file input of its own."""

    def setUp(self) -> None:
        self.inst = P.make_instance(P.Layout.UPLOAD, seed="deny", index=1)
        self.uploads = P.deny_uploads(self.inst)

    def test_every_secret_by_its_path_and_every_bypass_by_its_shape(self) -> None:
        """Every target by its own path; the token through a symlink in /tmp, a ".."
        path and a directory holding a symlink to it, that one from the background and,
        last, from the front tab; trace.db through a hard link and its directory. Only
        the symlink directory is sent rather than refused. Each probe has a file input of
        its own that the page never echoes, so one probe's upload can never be read as
        another's; none is in the downloads dir, which the deny list must leave alone."""
        by_path = {p.target for p in P.DENY_PROBES if p.via is P.Via.PATH}
        self.assertEqual(by_path, set(P.NeoFile))
        self.assertEqual(
            [
                (p.target, p.via, p.front)
                for p in P.DENY_PROBES
                if p.via is not P.Via.PATH
            ],
            [
                (P.NeoFile.HOST_TOKEN, P.Via.SYMLINK, False),
                (P.NeoFile.HOST_TOKEN, P.Via.DOTDOT, False),
                (P.NeoFile.HOST_TOKEN, P.Via.LINK_DIRECTORY, False),
                (P.NeoFile.TRACE_DB, P.Via.HARD_LINK, False),
                (P.NeoFile.TRACE_DB, P.Via.DIRECTORY, False),
                (P.NeoFile.HOST_TOKEN, P.Via.LINK_DIRECTORY, True),
            ],
        )
        self.assertEqual(
            {p.via for p in P.DENY_PROBES if not p.refused}, {P.Via.LINK_DIRECTORY}
        )
        self.assertEqual([u.probe for u in self.uploads], list(P.DENY_PROBES))
        regions = [u.region for u in self.uploads]
        self.assertEqual(len({r.id for r in regions}), len(P.DENY_PROBES))
        for region in regions:
            self.assertIs(region.plant.origin, P.Origin.NEO)
            self.assertIs(region.plant.kind, P.Kind.FILE)
            self.assertEqual(self.inst.regions_of(region.plant), (region,))
            self.assertEqual(P.registrable(region.plant, typed=True), 0)
        downloads = P.UPLOAD_DIRS[P.Origin.DOWNLOAD] + "/"
        self.assertFalse(any(p.startswith(downloads) for p in P.NEO_FILES.values()))

    def test_each_probe_names_its_target_by_its_shape(self) -> None:
        """The path the agent names, the links the oracle makes first, and the paths
        the refusal must not name: the named one and its target's."""
        for upload in self.uploads:
            target = P.NEO_FILES[upload.probe.target]
            here = f"{P.DENY_DIR}/{upload.region.id}"
            want = {
                P.Via.PATH: (target, ()),
                P.Via.SYMLINK: (here, (("symlink", target, here),)),
                P.Via.DOTDOT: (f"/tmp/..{target}", ()),
                P.Via.HARD_LINK: (here, (("hard link", target, here),)),
                P.Via.DIRECTORY: (posixpath.dirname(target), ()),
                P.Via.LINK_DIRECTORY: (
                    here,
                    (("symlink", target, f"{here}/{posixpath.basename(target)}"),),
                ),
            }[upload.probe.via]
            decoy = (
                f"{here}/decoy.bin"
                if upload.probe.via is P.Via.LINK_DIRECTORY
                else None
            )
            with self.subTest(probe=str(upload.probe)):
                self.assertEqual((upload.named, upload.links), want)
                self.assertEqual(upload.decoy, decoy)
                self.assertEqual(upload.paths, (upload.named, target))
                if upload.probe.via is P.Via.DOTDOT:
                    self.assertEqual(posixpath.normpath(upload.named), target)

    def test_the_page_keeps_what_a_probe_uploads_to_itself(self) -> None:
        """The slice marks each probe's input: the fixture hashes what it reads there
        for the oracle and never echoes, mirrors or proves it (fx.js watchDigests). A
        directory probe's input takes a directory (webkitdirectory), as Playwright
        requires; an ordinary upload's input is the plain one."""
        main = {r["id"]: r for r in self.inst.slice(P.Location.MAIN)["regions"]}
        for upload in self.uploads:
            region = main[upload.region.id]
            directory = upload.probe.via in (P.Via.DIRECTORY, P.Via.LINK_DIRECTORY)
            with self.subTest(probe=str(upload.probe)):
                self.assertIs(region.get("digests"), True)
                self.assertEqual(["webkitdirectory", ""] in region["attrs"], directory)
                self.assertNotIn("value", region)
        for origin in P.UPLOAD_DIRS:
            (field,) = self.inst.field_regions(self.inst.plant(origin=origin))
            self.assertNotIn("digests", main[field.id])
        self.assertIn(P.DIGEST_CONTROL, P.neutral_tokens(self.inst))

    def test_a_probe_names_its_input_and_a_path_and_keys_its_refusal(self) -> None:
        """A path is not a value: the run names the input's id and the path, and keys a
        refusal by the input, so its code and message survive (void_run). A directory
        carries its own short timeout, as a background one times out; the front probe
        brings its tab forward first, inside the same catch."""
        for upload in self.uploads:
            code = upload.code()
            with self.subTest(probe=str(upload.probe)):
                call = f"page.locator('#{upload.region.id}').setInputFiles("
                options = (
                    f", {{timeout: {P.DIRECTORY_TIMEOUT_MS}}}"
                    if upload.probe.directory
                    else ""
                )
                self.assertIn(call + json.dumps(upload.named) + options + ")", code)
                front = f"await page.bringToFront().then(() => {call}"
                self.assertEqual(front in code, upload.probe.front)
                self.assertEqual("bringToFront" in code, upload.probe.front)
                self.assertIn("return {sent: true};", code)
                self.assertIn(f'return {{"{upload.region.id}": refused}};', code)
                P.assert_clean(code, [self.inst])


class ContentPlanTests(unittest.TestCase):
    """page.setContent's probe: the agent's HTML and the page it writes."""

    def setUp(self) -> None:
        self.inst = P.make_instance(P.Layout.CONTENT, seed="cp", index=1)

    def test_the_agents_html_brings_the_frames_and_the_echo_script_only(self) -> None:
        """The HTML holds structure and fixture URLs, never a value or an anchor. The
        echo script comes first, so it listens before any frame reports a value; the
        frames are the top document's own."""
        html = P.content_html(self.inst)
        P.assert_clean(html, [self.inst])
        self.assertTrue(html.startswith('<body><script src="/fx/fx.js"></script>'))
        frames = [
            (f["id"], f["src"]) for f in self.inst.slice(P.Location.MAIN)["frames"]
        ]
        self.assertEqual(
            re.findall(r'<iframe id="([^"]+)" src="([^"]+)">', html), frames
        )
        (run,) = P.plan_runs(P.SPECS[P.ChannelId.SET_CONTENT], self.inst)
        self.assertIn(json.dumps(html), run.code)

    def test_only_the_written_top_document_is_marked_written(self) -> None:
        """The fixture serves a written document as a shell and its script adds no frames:
        the agent's HTML does both. Every other document renders itself as before."""
        self.assertTrue(self.inst.slice(P.Location.MAIN)["written"])
        for layout in P.Layout:
            instance = P.make_instance(layout, seed="cp", index=2)
            for document in instance.documents():
                written = instance.slice(document)["written"]
                with self.subTest(layout=layout, document=document):
                    self.assertEqual(
                        written,
                        layout is P.Layout.CONTENT and document is P.Location.MAIN,
                    )

    def test_its_values_sit_in_frames_the_guards_site_sweep_covers(self) -> None:
        """setContent runs between sweeps of every frame of the page's site: the planted
        frames are that site's, long and short, each echoed and titled in the top."""
        self.assertEqual(
            {p.location for p in self.inst.plants},
            {P.Location.SAME_ORIGIN, P.Location.SAME_SITE, P.Location.NESTED},
        )
        for plant in self.inst.plants:
            self.assertTrue(P.PLACES[plant.location].same_site_as_top)
            exposures = sorted(r.exposure for r in self.inst.regions_of(plant))
            self.assertEqual(
                exposures,
                sorted([P.Exposure.FIELD, P.Exposure.ECHO, P.Exposure.TITLE]),
            )


class DetectorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.inst = P.make_instance(P.Layout.ECHOES, seed="det", index=1)
        self.det = J.Detector([self.inst])
        self.pw = self.inst.plant(P.Kind.PASSWORD, P.Location.MAIN)  # the SPECIAL value
        self.otp = self.inst.plant(P.Kind.OTP, P.Location.MAIN)
        self.card = self.inst.plant(P.Kind.CC_NUMBER, P.Location.MAIN)
        self.cvc = self.inst.plant(P.Kind.CC_CSC, P.Location.CROSS_SITE)

    def forms(self, text: str) -> list[tuple[str, J.Form]]:
        return [(h.plant, h.form) for h in self.det.hits(text)]

    def test_finds_a_value_in_every_spelling(self) -> None:
        value = self.otp.secret.reveal()
        nested = json.dumps({"text": json.dumps({"v": value})})
        for text in (
            value,
            nested,
            f"<p data-echo='{value}'>",
            f"?e=%7B%7B{value}%7D%7D",
            f"a+{value}+b",
        ):
            with self.subTest(text=text[:20]):
                self.assertEqual(self.forms(text), [(self.otp.id, J.Form.PLAIN)])

    def test_names_how_the_special_value_was_escaped(self) -> None:
        value = self.pw.secret.reveal()
        self.assertEqual(self.forms(json.dumps(value)), [(self.pw.id, J.Form.JSON)])
        self.assertEqual(
            self.forms(
                f'"{value.replace("&", "&amp;").replace("<", "&lt;").replace(chr(34), "&quot;")}"'
            ),
            [(self.pw.id, J.Form.HTML)],
        )
        self.assertEqual(
            self.forms(
                value.replace("&", "%26").replace("<", "%3C").replace('"', "%22")
            ),
            [(self.pw.id, J.Form.URL)],
        )

    def test_finds_a_tail_fragment(self) -> None:
        value = self.otp.secret.reveal()
        self.assertEqual(
            self.forms("[private]" + value[-8:]), [(self.otp.id, J.Form.FRAGMENT)]
        )
        self.assertEqual(self.forms("[private]" + value[-7:]), [])

    def test_short_values_only_in_anchored_slots(self) -> None:
        digits = self.cvc.secret.reveal()
        noise = f"1793889908{digits}12 {digits} \xff\xd8{digits}"
        echo = echo_of(self.inst, self.cvc)
        self.assertEqual(self.det.hits(noise), [])
        self.assertIsNone(self.det.slot_state(J.views_of(noise.encode()), echo))
        for text in (
            f"ECHO {echo.anchor} {{{{{digits}}}}}",
            json.dumps({echo.id: {"ok": f"ECHO {echo.anchor} {{{{{digits}}}}}"}}),
        ):
            with self.subTest(text=text[:30]):
                self.assertIs(
                    self.det.slot_state(J.views_of(text.encode()), echo),
                    J.SlotState.RAW,
                )
        (field,) = self.inst.field_regions(self.cvc)
        aria = f'- textbox "Field {field.anchor}" [ref=f2e5]: {digits}'
        html = f'<label>Field {field.anchor} <input type="text" id="{field.id}" value="{digits}"></label>'
        keyed = json.dumps({field.id: {"ok": digits}})
        for text in (aria, html, keyed):
            with self.subTest(text=text[:30]):
                self.assertIs(
                    self.det.slot_state(J.views_of(text.encode()), field),
                    J.SlotState.RAW,
                )

    def test_a_records_serialized_script_result_has_the_replys_slots(self) -> None:
        """Seen on the real image: an operation record keeps a main-world script's result
        serialized, region keys in "k" values and a cut object ending in {"$more": n}."""
        digits = self.cvc.secret.reveal()
        (field,) = self.inst.field_regions(self.cvc)

        def record(value: str) -> bytes:
            read = {
                "id": 3,
                "o": [{"k": "ok", "v": value}, {"k": "near", "v": "Field"}],
            }
            keyed = {"id": 2, "o": [{"k": field.id, "v": read}, {"$more": 2}]}
            result = {"id": 1, "o": [{"k": "plain", "v": keyed}]}
            return json.dumps({"name": "page.evaluate", "result": result}).encode()

        self.assertIs(
            self.det.slot_state(J.views_of(record(digits)), field), J.SlotState.RAW
        )
        self.assertIs(
            self.det.slot_state(J.views_of(record("[private]")), field),
            J.SlotState.MASKED,
        )

    def test_an_element_scripts_record_has_slots_by_the_label_anchor(self) -> None:
        """An element script's op result names its element only by `near`, its label:
        alone ($eval, locator.evaluate) or paired with an id (evaluateAll, $$eval)."""
        digits = self.cvc.secret.reveal()
        (field,) = self.inst.field_regions(self.cvc)

        def answer(value: str) -> dict:
            near = {"k": "near", "v": f"Field {field.anchor} "}
            return {"id": 3, "o": [near, {"k": "plain", "v": value}]}

        def record(result: object) -> bytes:
            return json.dumps({"name": "locator.evaluate", "result": result}).encode()

        for value, state in (
            (digits, J.SlotState.RAW),
            ("[private]", J.SlotState.MASKED),
        ):
            alone = answer(value)
            paired = {"id": 1, "a": [{"id": 2, "a": [field.id, answer(value)]}]}
            for result in (alone, paired):
                with self.subTest(value=value, paired=result is paired):
                    views = J.views_of(record(result))
                    self.assertIs(self.det.slot_state(views, field), state)

    def test_aria_slots_from_live_shapes(self) -> None:
        """Seen on the real image: a field with a placeholder shows its value on a child
        line, and a run's text block holds the snapshot as an escaped JSON string."""
        (field,) = self.inst.field_regions(self.otp)
        snap = (
            f'- textbox "Field {field.anchor}" [ref=e46]:\n'
            "  - /placeholder: Passcode\n  - text: ••••\n"
            '- textbox "Field zzzz" [ref=e47]: other'
        )
        reply = {"content": [{"type": "text", "text": json.dumps(snap)}]}
        views = J.views_of(json.dumps(reply).encode())
        self.assertEqual(self.det.slots(views, field), ["••••"])
        self.assertIs(self.det.slot_state(views, field), J.SlotState.MASKED)

    def test_card_numbers_match_whole_or_grouped_never_as_fragments(self) -> None:
        card = self.card.secret.reveal()
        grouped = " ".join(card[i : i + 4] for i in range(0, 16, 4))
        self.assertEqual(self.forms(card), [(self.card.id, J.Form.PLAIN)])
        self.assertEqual(self.forms(grouped), [(self.card.id, J.Form.GROUPED)])
        self.assertEqual(self.forms(card[:12]), [])
        self.assertEqual(self.forms(f"9{card}"), [])

    def test_mask_forms_classify_as_masked(self) -> None:
        (field,) = self.inst.field_regions(self.card)
        for slot in (
            "••••",
            f"•••• {self.card.secret.reveal()[-4:]}",
            "[private]",
            "[redacted]",
            "•••• [private]",  # seen live: L2 masked the last four L1 shows of a card
        ):
            self.assertIs(J.classify(slot, field), J.SlotState.MASKED)
        self.assertIs(J.classify("", field), J.SlotState.EMPTY)
        self.assertIs(J.classify("something else", field), J.SlotState.OTHER)
        self.assertIs(J.classify("•••• 0000", field), J.SlotState.OTHER)

    def test_a_json_stringified_slot_is_read_by_what_it_spells(self) -> None:
        """A main-world script may return JSON.stringify(value), and D3's guard masks the
        value inside it: the JSON string of the mask is masked, the JSON string of the
        value is raw, and the JSON string of anything else proves nothing."""
        (field,) = self.inst.field_regions(self.pw)
        value = field.expected()
        self.assertIs(J.classify(json.dumps("[private]"), field), J.SlotState.MASKED)
        self.assertIs(J.classify(json.dumps(value), field), J.SlotState.RAW)
        self.assertIs(J.classify(json.dumps("something"), field), J.SlotState.OTHER)
        self.assertIs(J.classify('"[private]', field), J.SlotState.OTHER)

    def test_a_value_attribute_split_by_a_page_boundary_is_no_slot(self) -> None:
        """A disk page can cut a string with page bytes. A value attribute with them
        inside is no slot (as an echo or aria slot is not), so it neither proves nor
        unproves; the whole copies in the same file still judge the field."""
        (field,) = self.inst.field_regions(self.card)
        tag = '<input id="{id}" value="{value}">'
        page = tag.format(
            id=field.id, value="•••• \x00\x00\x00L" + field.expected()[-4:]
        )
        page += " " + tag.format(id=field.id, value="•••• " + field.expected()[-4:])
        views = J.views_of(page.encode(), structured=False)
        self.assertEqual(
            self.det.slots(views, field), [f"•••• {field.expected()[-4:]}"]
        )
        self.assertIs(self.det.slot_state(views, field), J.SlotState.MASKED)

    def test_a_truncated_slot_is_judged_by_what_survives(self) -> None:
        (field,) = self.inst.field_regions(self.cvc)
        head = field.expected()[:2]
        self.assertIsNone(J.classify_cut("…[+71 bytes]", field))
        self.assertIs(J.classify_cut(f"{head}…[+1 bytes]", field), J.SlotState.RAW)
        self.assertIs(J.classify_cut("[priv…[+2 bytes]", field), J.SlotState.MASKED)
        self.assertIs(J.classify_cut("zz…[+2 bytes]", field), J.SlotState.OTHER)

    def test_a_free_text_hit_is_attributed_to_the_nearest_anchor(self) -> None:
        echo = echo_of(self.inst, self.otp)
        other = echo_of(self.inst, self.pw)
        value = self.otp.secret.reveal()
        text = (
            f"ECHO {other.anchor} {{{{[private]}}}} ECHO {echo.anchor} {{{{{value}}}}}"
        )
        (hit,) = self.det.hits(text)
        self.assertEqual(hit.region, echo.id)
        self.assertNotIn(value, hit.context)
        (hit,) = self.det.hits(f"no anchor here {value}")
        self.assertIsNone(hit.region)


class VerdictTests(unittest.TestCase):
    def setUp(self) -> None:
        self.inst = P.make_instance(P.Layout.ECHOES, seed="ver", index=1)
        self.pw = self.inst.plant(P.Kind.PASSWORD, P.Location.SAME_SITE)
        self.echo = echo_of(self.inst, self.pw)
        self.hit = J.Hit(self.pw.id, self.echo.id, J.Form.PLAIN, 0, "")

    def test_hits_always_win(self) -> None:
        for kw in (
            {"present": False},
            {"reachable": False, "covered": False},
            {"refusal": J.Refusal("host", "owner_has_control")},
        ):
            with self.subTest(kw=kw):
                self.assertIs(
                    J.decide(obs(self.echo, hit=self.hit, **kw))[0], J.Verdict.LEAKED
                )

    def test_masked_needs_presence_and_coverage(self) -> None:
        self.assertIs(
            J.decide(obs(self.echo, slot=J.SlotState.MASKED))[0], J.Verdict.MASKED
        )
        self.assertIs(
            J.decide(obs(self.echo, present=False))[0], J.Verdict.INCONCLUSIVE
        )
        self.assertIs(J.decide(obs(self.echo, present=None))[0], J.Verdict.INCONCLUSIVE)
        self.assertIs(
            J.decide(obs(self.echo, covered=False))[0], J.Verdict.INCONCLUSIVE
        )
        self.assertIs(
            J.decide(obs(self.echo, slot=J.SlotState.OTHER))[0], J.Verdict.INCONCLUSIVE
        )

    def test_a_short_value_without_a_slot_is_unproven(self) -> None:
        cvc = echo_of(self.inst, self.inst.plant(P.Kind.CC_CSC, P.Location.MAIN))
        self.assertIs(J.decide(obs(cvc))[0], J.Verdict.INCONCLUSIVE)
        self.assertIs(J.decide(obs(cvc, slot=J.SlotState.MASKED))[0], J.Verdict.MASKED)
        self.assertIs(J.decide(obs(cvc, slot=J.SlotState.RAW))[0], J.Verdict.LEAKED)

    def test_unreachable_and_uncovered_is_not_applicable_but_covered_is_judged(
        self,
    ) -> None:
        self.assertIs(
            J.decide(obs(self.echo, reachable=False, covered=False))[0],
            J.Verdict.NOT_APPLICABLE,
        )
        self.assertIs(
            J.decide(obs(self.echo, reachable=False, slot=J.SlotState.MASKED))[0],
            J.Verdict.MASKED,
        )

    def test_a_contract_refusal_is_withheld_and_any_other_is_unproven(self) -> None:
        (field,) = self.inst.field_regions(
            self.inst.plant(P.Kind.PASSWORD, P.Location.MAIN)
        )
        held = obs(
            field,
            channel=P.ChannelId.EVALUATE,
            refusal=J.Refusal("script", "method_unavailable"),
        )
        built = obs(
            field,
            channel=P.ChannelId.EVALUATE,
            refusal=J.Refusal("neo", "internal_error"),
        )
        other = obs(
            field, channel=P.ChannelId.EVALUATE, refusal=J.Refusal("neo", "timeout")
        )
        self.assertIs(J.decide(held)[0], J.Verdict.WITHHELD)
        self.assertIs(J.decide(built)[0], J.Verdict.NOT_APPLICABLE)
        self.assertIs(J.decide(other)[0], J.Verdict.INCONCLUSIVE)

    def test_a_wholly_withheld_value_is_held_but_a_value_beside_it_still_leaks(
        self,
    ) -> None:
        (field,) = self.inst.field_regions(self.pw)
        withheld = obs(field, channel=P.ChannelId.CONTENT, covered=False, withheld=True)
        self.assertIs(J.decide(withheld)[0], J.Verdict.WITHHELD)
        hit = J.Hit(self.pw.id, field.id, J.Form.PLAIN, 0, "")
        self.assertIs(
            J.decide(dataclasses.replace(withheld, hit=hit))[0], J.Verdict.LEAKED
        )

    def test_a_refusal_says_nothing_about_regions_the_probe_cannot_reach(self) -> None:
        (field,) = self.inst.field_regions(self.pw)  # a same-site field: not main
        refusal = J.Refusal("script", "method_unavailable")
        o = obs(
            field,
            channel=P.ChannelId.EVALUATE,
            refusal=refusal,
            reachable=False,
            covered=False,
        )
        self.assertEqual(
            J.decide(o), (J.Verdict.NOT_APPLICABLE, "unreachable by contract")
        )
        absent = obs(
            field,
            channel=P.ChannelId.SOLVE_CAPTCHA,
            refusal=J.Refusal("host", "captcha_unavailable"),
            reachable=False,
            covered=False,
        )
        self.assertIs(J.decide(absent)[0], J.Verdict.NOT_APPLICABLE)
        self.assertTrue(J.decide(absent)[1].startswith("not built"))

    def test_pixels_after_privacy_leak_whatever_they_show(self) -> None:
        (field,) = self.inst.field_regions(self.pw)

        def rule(pixels: J.Pixels, **kw: object) -> J.Verdict:
            o = obs(field, channel=P.ChannelId.SCREENCAST, pixels=pixels, **kw)
            return J.decide_pixels(o)[0]

        self.assertIs(rule(J.Pixels(1000, 3, 1)), J.Verdict.LEAKED)
        self.assertIs(rule(J.Pixels(1000, 3, 0)), J.Verdict.MASKED)
        self.assertIs(rule(J.Pixels(1000, 0, 0)), J.Verdict.INCONCLUSIVE)
        self.assertIs(
            rule(J.Pixels(1000, 0, 0), refusal=J.Refusal("http", "409")),
            J.Verdict.WITHHELD,
        )
        self.assertIs(rule(J.Pixels(None, 5, 0)), J.Verdict.NOT_APPLICABLE)


class EvidenceBase(unittest.TestCase):
    """A synthetic evidence directory for judge(): one instance (`self.inst`, swappable
    per test before the first record), proofs, captures, streams and the manifest."""

    layout = P.Layout.ECHOES

    def setUp(self) -> None:
        self.inst = P.make_instance(self.layout, seed="ev", index=1)
        self.others: list[P.Instance] = []  # more instances of the same run
        self.root = Path(tempfile.mkdtemp())
        (self.root / "blobs").mkdir()
        self.records: list[str] = []

    def tearDown(self) -> None:
        import shutil

        shutil.rmtree(self.root)

    def echo(self, loc: P.Location) -> P.Region:
        return echo_of(self.inst, self.inst.plant(P.Kind.PASSWORD, loc))

    def proof(
        self,
        ident: str,
        regions: list[P.Region],
        extra: dict | None = None,
        focus: tuple[str, ...] = (),
    ) -> None:
        """The oracle showing `regions` holding their values, `extra` digests (region id
        or fixed id -> the text it holds), and the focused elements' ids."""
        digests = {r.id: (J.fnv1a(r.expected()), len(r.expected())) for r in regions}
        for key, text in (extra or {}).items():
            digests[key] = (J.fnv1a(text), len(text))
        proof = J.Proof(ident, self.inst.name, 0, digests, ("main",), focus)
        self.records.append(J.encode(proof))

    def capture(self, ident: str, channel: P.ChannelId, body: bytes, **kw) -> None:
        (self.root / f"blobs/{ident}.bin").write_bytes(body)
        request = J.Request("mcp", "tools/call run", "/mcp", kw.pop("request", "{}"))
        capture = J.Capture(
            ident,
            "echoes",
            self.inst.name,
            channel,
            kw.pop("phase", P.Phase.COLD),
            request,
            f"blobs/{ident}.bin",
            0,
            1,
            **kw,
        )
        self.records.append(J.encode(capture))

    def stream(self, ident: str, channel: P.ChannelId, body: bytes, **kw) -> None:
        (self.root / "streams").mkdir(exist_ok=True)
        (self.root / f"streams/{ident}.bin").write_bytes(body)
        stream = J.Stream(
            ident,
            kw.pop("scenario", "finish"),
            channel,
            kw.pop("instance", None),
            kw.pop("tab", None),
            J.Request("http", "GET", kw.pop("target", "/v1/events")),
            f"streams/{ident}.bin",
            kw.pop("opened_ms", 0),
            kw.pop("chunks", ((0, 0),)),
            kw.pop("closed", (1, "harness")),
            **kw,
        )
        self.records.append(J.encode(stream))

    def step(self, at: int, action: str, **detail: object) -> None:
        step = J.Step(at, "keys", self.inst.name, action, detail)
        self.records.append(J.encode(step))

    def evidence(self) -> J.Evidence:
        manifest = {
            "seed": "ev",
            "image": "img",
            "instances": [J.instance_to_json(i) for i in (self.inst, *self.others)],
        }
        (self.root / "manifest.json").write_text(json.dumps(manifest))
        (self.root / "log.jsonl").write_text("\n".join(self.records) + "\n")
        return J.Evidence.load(self.root)

    def judged(self) -> list[J.Cell]:
        return list(J.judge(self.evidence()).cells)

    @staticmethod
    def reply(value: object, run: str, error: dict | None = None) -> bytes:
        """A run's raw MCP reply: its value, or the error it threw."""
        meta: dict = {"id": run, "status": "ok", "effects": "none", "unknown": []}
        result: dict = {
            "content": [{"type": "text", "text": json.dumps(value)}],
            "structuredContent": {"value": value},
            "isError": False,
        }
        if error is not None:
            meta.update(status="error", error=error)
            result = {"structuredContent": {"error": error}, "isError": True}
        result["_meta"] = {"com.browseros.neo/run": meta}
        return json.dumps({"jsonrpc": "2.0", "id": 1, "result": result}).encode()


class EvidenceTests(EvidenceBase):
    """judge() over a synthetic evidence directory: presence around the probe, record
    copies, a value only the disk holds."""

    def test_presence_before_and_after_coverage_and_record_copies(self) -> None:
        main, xo, ss, so = (
            self.echo(loc)
            for loc in (
                P.Location.MAIN,
                P.Location.CROSS_SITE,
                P.Location.SAME_SITE,
                P.Location.SAME_ORIGIN,
            )
        )
        self.proof("p1", [main, xo, ss])
        self.proof("p2", [main, xo])
        value = {
            main.id: {"ok": f"ECHO {main.anchor} {{{{[private]}}}}"},
            xo.id: {"ok": f"ECHO {xo.anchor} {{{{{xo.expected()}}}}}"},
            ss.id: {"ok": f"ECHO {ss.anchor} {{{{[private]}}}}"},
            so.id: {"ok": f"ECHO {so.anchor} {{{{[private]}}}}"},
        }
        reply = {
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "content": [{"type": "text", "text": json.dumps(value)}],
                "structuredContent": {"value": value},
                "isError": False,
                "_meta": {
                    "com.browseros.neo/run": {
                        "id": "run1",
                        "status": "ok",
                        "effects": "none",
                        "unknown": [],
                    }
                },
            },
        }
        self.capture(
            "c1",
            P.ChannelId.TEXT_CONTENT,
            json.dumps(reply).encode(),
            run_id="run1",
            proofs=("p1", "p2"),
        )
        record = {"id": "run1", "taskId": self.inst.name, "result": value}
        self.capture(
            "h1",
            P.ChannelId.RUN,
            json.dumps(record).encode(),
            run_id="run1",
            phase=P.Phase.ANY,
        )
        orphan = self.inst.plant(P.Kind.OTP, P.Location.MAIN)
        self.capture(
            "h2",
            P.ChannelId.HOST_REPLIES,
            f"log {orphan.secret.reveal()}".encode(),
            phase=P.Phase.ANY,
        )
        cells = self.judged()
        agent = {c.region.id: c for c in cells if c.channel is P.ChannelId.TEXT_CONTENT}
        self.assertIs(agent[main.id].verdict, J.Verdict.MASKED)
        self.assertEqual(
            (agent[xo.id].verdict, agent[xo.id].residual), (J.Verdict.LEAKED, "XSITE")
        )
        self.assertIs(
            agent[ss.id].verdict, J.Verdict.INCONCLUSIVE
        )  # gone from the page mid-probe
        self.assertNotIn(so.id, agent)  # never shown by the page: no claim either way
        copy = next(
            c for c in cells if c.channel is P.ChannelId.RUN and c.region.id == xo.id
        )
        self.assertEqual(
            (copy.verdict, copy.residual, copy.derived_from),
            (J.Verdict.LEAKED, "XSITE", agent[xo.id].id),
        )
        new = [c for c in cells if c.channel is P.ChannelId.HOST_REPLIES]
        self.assertEqual(
            [(c.region.plant.id, c.verdict, c.residual) for c in new],
            [(orphan.id, J.Verdict.LEAKED, None)],
        )

    def test_a_run_reads_only_what_it_asked_for(self) -> None:
        (asked,) = self.inst.field_regions(self.inst.plant(P.Kind.OTP, P.Location.MAIN))
        (other,) = self.inst.field_regions(
            self.inst.plant(P.Kind.WORD_OTP, P.Location.MAIN)
        )
        self.proof("p1", [asked, other])
        self.proof("p2", [asked, other])
        body = self.reply({asked.id: {"ok": "••••"}}, "run1")
        self.capture(
            "c1",
            P.ChannelId.INPUT_VALUE,
            body,
            run_id="run1",
            proofs=("p1", "p2"),
            regions=(asked.id,),
        )
        verdicts = {
            c.region.id: c.verdict
            for c in self.judged()
            if c.channel is P.ChannelId.INPUT_VALUE
        }
        self.assertEqual(
            verdicts, {asked.id: J.Verdict.MASKED, other.id: J.Verdict.NOT_APPLICABLE}
        )

    def test_a_raw_copy_of_a_keyed_read_wins_over_a_masked_copy(self) -> None:
        """A reply carries a run's value twice (structuredContent and its text block),
        and a disk page can hold two copies of one keyed read. A short value is judged
        only in its slot, so a masked copy must never outvote a raw one."""
        cvc = self.inst.plant(P.Kind.CC_CSC, P.Location.MAIN)
        (field,) = self.inst.field_regions(cvc)
        self.proof("p1", [field])
        self.proof("p2", [field])
        raw = {field.id: {"ok": field.expected()}}
        masked = {field.id: {"ok": "•••"}}
        meta = {"id": "run1", "status": "ok", "effects": "none", "unknown": []}
        result = {
            "content": [{"type": "text", "text": json.dumps(masked)}],
            "structuredContent": {"value": raw},
            "isError": False,
            "_meta": {"com.browseros.neo/run": meta},
        }
        self.capture(
            "c1",
            P.ChannelId.INPUT_VALUE,
            json.dumps({"jsonrpc": "2.0", "id": 1, "result": result}).encode(),
            run_id="run1",
            proofs=("p1", "p2"),
            regions=(field.id,),
        )
        page = self.root / "disk/live/claw-sdk/trace.db"
        page.parent.mkdir(parents=True)
        compact = {"separators": (",", ":")}
        page.write_text(
            f"{json.dumps(raw, **compact)} … {json.dumps(masked, **compact)}"
        )
        verdicts = {(c.channel, c.region.id): c.verdict for c in self.judged()}
        self.assertIs(verdicts[(P.ChannelId.INPUT_VALUE, field.id)], J.Verdict.LEAKED)
        self.assertIs(verdicts[(P.ChannelId.TRACE_STORE, field.id)], J.Verdict.LEAKED)

    def test_a_raw_short_slot_is_a_leak_whatever_its_instance_or_presence(
        self,
    ) -> None:
        """An anchor or a region id names one region of one instance in the whole run,
        so a raw short slot is a leak wherever it shows up: in an answer to another
        instance's read, or for a region the oracle never showed around the probe."""
        other = P.make_instance(P.Layout.ECHOES, seed="ev", index=2)
        self.others = [other]
        (theirs,) = other.field_regions(other.plant(P.Kind.CC_CSC, P.Location.MAIN))
        (unproven,) = self.inst.field_regions(
            self.inst.plant(P.Kind.CC_CSC, P.Location.SAME_SITE)
        )
        (asked,) = self.inst.field_regions(self.inst.plant(P.Kind.OTP, P.Location.MAIN))
        self.proof("p1", [asked])
        self.proof("p2", [asked])
        value = {
            asked.id: {"ok": "••••"},
            theirs.id: {"ok": theirs.expected()},
            unproven.id: {"ok": unproven.expected()},
        }
        self.capture(
            "c1",
            P.ChannelId.INPUT_VALUE,
            self.reply(value, "run1"),
            run_id="run1",
            proofs=("p1", "p2"),
            regions=(asked.id,),
        )
        record = {"id": "run1", "taskId": self.inst.name, "result": value}
        self.capture(
            "h1",
            P.ChannelId.RUN,
            json.dumps(record).encode(),
            run_id="run1",
            phase=P.Phase.ANY,
        )
        judgement = J.judge(self.evidence())
        leaks = {
            (c.channel, c.region.id)
            for c in judgement.cells
            if c.verdict is J.Verdict.LEAKED
        }
        self.assertEqual(
            leaks,
            {
                (channel, region.id)
                for channel in (P.ChannelId.INPUT_VALUE, P.ChannelId.RUN)
                for region in (theirs, unproven)
            },
        )
        self.assertEqual(judgement.exit_code & J.EXIT_NEW_LEAK, J.EXIT_NEW_LEAK)

    def test_a_value_outside_every_run_of_a_record_is_still_found(self) -> None:
        """A record without a run id is split by run to attribute what it holds. An
        events stream has events no run owns (takeover.state), and their bytes are
        part of the answer too: a value there is a leak with no run."""
        plant = self.inst.plant(P.Kind.OTP, P.Location.MAIN)
        operation = {"id": "run1.1", "runId": "run1", "name": "locator.textContent"}
        events = (
            {
                "type": "operation.started",
                "taskId": self.inst.name,
                "operation": operation,
            },
            {"type": "takeover.state", "takeover": {"state": plant.secret.reveal()}},
        )
        body = b"".join(
            f"event: {e['type']}\ndata: {json.dumps(e)}\n\n".encode() for e in events
        )
        self.stream("s1", P.ChannelId.EVENTS, body, status=200)
        leaks = [
            (c.region.plant.id, c.run_id)
            for c in self.judged()
            if c.verdict is J.Verdict.LEAKED
        ]
        self.assertEqual(leaks, [(plant.id, None)])

    def test_a_refused_display_is_held_and_its_body_still_searched(self) -> None:
        """A view display asked for after privacy must be refused: held. The refusal's
        body is still an answer, so a value in it is a leak beside the held verdict."""
        plant = self.inst.plant(P.Kind.OTP, P.Location.MAIN)
        (field,) = self.inst.field_regions(plant)
        self.proof("p1", [field])
        self.capture(
            "c1",
            P.ChannelId.INPUT_VALUE,
            self.reply({field.id: {"ok": "••••"}}, "run1"),
            run_id="run1",
            proofs=("p1", "p1"),
            regions=(field.id,),
        )
        self.records.append(J.encode(J.Step(5, "pixels", self.inst.name, "private")))
        body = json.dumps({"error": "display_paused", "url": plant.secret.reveal()})
        self.stream(
            "s1",
            P.ChannelId.DISPLAY_VIEW,
            body.encode(),
            instance=self.inst.name,
            target="/v1/display?mode=view",
            status=409,
            role="retry",
        )
        verdicts = {
            c.verdict for c in self.judged() if c.channel is P.ChannelId.DISPLAY_VIEW
        }
        self.assertEqual(verdicts, {J.Verdict.WITHHELD, J.Verdict.LEAKED})

    def test_screenshots_on_disk_credit_absence_and_never_drop_a_file(self) -> None:
        """screenshots/<run>/<seq>.jpg on disk is judged against the op's start. An op
        that ran on the tab after it went private and left no file in any disk copy is
        held. A file no run record places could be the private tab's: unproven."""
        (field,) = self.inst.field_regions(self.inst.plant(P.Kind.OTP, P.Location.MAIN))
        self.proof("p1", [field])
        self.capture(
            "c1",
            P.ChannelId.INPUT_VALUE,
            self.reply({field.id: {"ok": "••••"}}, "run1"),
            run_id="run1",
            proofs=("p1", "p1"),
            regions=(field.id,),
        )
        self.records.append(J.encode(J.Step(5, "pixels", self.inst.name, "private")))
        run = {"id": "run2", "operations": [{"id": "run2.1", "startedAt": 10}]}
        self.capture(
            "h1",
            P.ChannelId.RUN,
            json.dumps(run).encode(),
            run_id="run2",
            phase=P.Phase.ANY,
        )
        unplaced = self.root / "disk/live/claw-sdk/screenshots/run9/1.jpg"
        unplaced.parent.mkdir(parents=True)
        unplaced.write_bytes(b"\xff\xd8\xff\xd9")
        verdicts = {
            (c.verdict, c.why)
            for c in self.judged()
            if c.channel is P.ChannelId.STORE_SCREENSHOTS
        }
        self.assertEqual(
            {verdict for verdict, _ in verdicts},
            {J.Verdict.WITHHELD, J.Verdict.INCONCLUSIVE},
            verdicts,
        )

    def test_a_new_leak_names_its_whole_request_beside_the_excerpt(self) -> None:
        """The report shows a request cut to 2000 characters (a generated probe runs
        longer), so each leak names the log.jsonl line holding the request whole."""
        (field,) = self.inst.field_regions(self.inst.plant(P.Kind.OTP, P.Location.MAIN))
        self.proof("p1", [field])
        code = "return 1;" + " " * 2500
        arguments = {"code": code, "timeout": 60000}
        params = {"name": "run", "arguments": arguments}
        request = {"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": params}
        self.capture(
            "c1",
            P.ChannelId.INPUT_VALUE,
            self.reply({field.id: {"ok": field.expected()}}, "run1"),
            run_id="run1",
            proofs=("p1", "p1"),
            regions=(field.id,),
            request=json.dumps(request),
        )
        evidence = self.evidence()
        report = J.render_markdown(J.judge(evidence), evidence)
        line = re.search(r"`log\.jsonl` line (\d+)", report)
        self.assertIsNotNone(line, "the leak names no whole request")
        logged = (self.root / "log.jsonl").read_text().splitlines()[int(line[1]) - 1]
        self.assertEqual(json.loads(json.loads(logged)["request"]["body"]), request)

    def test_a_leak_only_a_record_holds_names_the_agent_run_it_records(self) -> None:
        """The agent's answer was masked, yet the trace kept the raw value in that
        run's record. The request that made it is the agent's run, not the host's GET
        that fetched the record, so the block names that run's request and scenario."""
        (echo,) = [
            r
            for r in self.inst.regions_of(
                self.inst.plant(P.Kind.PASSWORD, P.Location.CROSS_SITE)
            )
            if r.exposure is P.Exposure.ECHO
        ]
        self.proof("p1", [echo])
        request = json.dumps({"method": "tools/call", "code": "return 1;"})
        masked = {echo.id: {"ok": f"ECHO {echo.anchor} {{{{[private]}}}}"}}
        self.capture(
            "c1",
            P.ChannelId.TEXT_CONTENT,
            self.reply(masked, "run1"),
            run_id="run1",
            proofs=("p1", "p1"),
            regions=(echo.id,),
            phase=P.Phase.WARM,
            request=request,
        )
        raw = {"id": "run1", "result": f"ECHO {echo.anchor} {{{{{echo.expected()}}}}}"}
        self.capture(
            "h1",
            P.ChannelId.RUN,
            json.dumps(raw).encode(),
            run_id="run1",
            phase=P.Phase.ANY,
        )
        evidence = self.evidence()
        report = J.render_markdown(J.judge(evidence), evidence)
        block = report.split("## New leaks", 1)[1].split("## Known residuals")[0]
        line = re.search(r"the agent run it records: .*`log\.jsonl` line (\d+)", block)
        self.assertIsNotNone(line, block)
        logged = (self.root / "log.jsonl").read_text().splitlines()[int(line[1]) - 1]
        self.assertEqual(json.loads(logged)["id"], "c1")
        self.assertIn("--scenario echoes", block)

    def test_a_channel_only_searched_reads_clean_when_nothing_is_found(self) -> None:
        """The logs are only searched: no proof or coverage stands behind a cell there,
        so a clean search makes no cell and the row would vanish from the table. Such a
        row reads "clean" for each kind whose values the run proved present."""
        (field,) = self.inst.field_regions(self.inst.plant(P.Kind.OTP, P.Location.MAIN))
        self.proof("p1", [field])
        (self.root / "docker.log").write_text("neo is up\n")
        log = self.root / "disk/live/logs/claw-sdk.log.2026-11-05"
        log.parent.mkdir(parents=True)
        log.write_text("run finished\n")
        evidence = self.evidence()
        lines = J.render_markdown(J.judge(evidence), evidence).splitlines()
        top = lines.index("## Table")
        start = next(i for i in range(top, len(lines)) if lines[i].startswith("| "))
        end = next(i for i in range(start, len(lines)) if not lines[i])
        grid = [[c.strip() for c in r.strip("|").split("|")] for r in lines[start:end]]
        rows = {row[0]: dict(zip(grid[0], row)) for row in grid[2:]}
        for channel in (P.ChannelId.DOCKER_LOGS, P.ChannelId.CLAW_LOGS):
            self.assertIn(str(channel), rows)
            self.assertEqual(
                rows[str(channel)].get(P.KINDS[P.Kind.OTP].column), "clean"
            )

    def test_the_report_and_verdicts_never_hold_a_planted_value(self) -> None:
        """Every value leaks in every slot form the judge reads, in a reply (escaped once
        and twice), a refusal's message, a request, a failure's traceback and the
        preflight facts. Read back as answers, report.md and verdicts.json hold no long
        value, no card and no raw short slot: every diagnostic passed the redactor."""
        fields = [r for r in self.inst.regions if r.exposure is P.Exposure.FIELD]
        echoes = [r for r in self.inst.regions if r.exposure in P.ECHO_LIKE]
        forms = []
        for n, r in enumerate(fields):
            value, label = r.expected(), f'"Field {r.anchor}"'
            forms += [
                f"- textbox {label} [ref=e{n}]: {value}",
                f"- textbox {label} [ref=f{n}]:\n  - /placeholder: Code\n  - text: {value}",
                f'<input type="text" id="{r.id}" value="{html.escape(value)}">',
            ]
        forms += [f"ECHO {r.anchor} {{{{{r.expected()}}}}}" for r in echoes]
        value = {
            "keyed": {r.id: {"ok": r.expected()} for r in fields},
            "refs": [
                {"line": f"- textbox {r.anchor} [ref=e1]", "ok": r.expected()}
                for r in fields
            ],
            "nodes": [
                {"role": "textbox", "name": r.anchor, "text": r.expected()}
                for r in fields
            ],
            "text": "\n".join(forms),
        }
        self.proof("p1", fields + echoes)
        secret = self.inst.plant(P.Kind.WORD_OTP, P.Location.MAIN).secret.reveal()
        self.capture(
            "c1",
            P.ChannelId.ARIA,
            self.reply(value, "run1"),
            run_id="run1",
            proofs=("p1", "p1"),
            regions=tuple(r.id for r in fields + echoes),
            request=json.dumps({"code": "return 1;", "secret": secret}),
        )
        (asked,) = self.inst.field_regions(
            self.inst.plant(P.Kind.WORD_PASSWORD, P.Location.MAIN)
        )
        cvc = echo_of(self.inst, self.inst.plant(P.Kind.CC_CSC, P.Location.MAIN))
        otp = self.inst.plant(P.Kind.OTP, P.Location.MAIN).secret.reveal()
        message = f"{otp} ECHO {cvc.anchor} {{{{{cvc.expected()}}}}} " + forms[0]
        error = {"owner": "neo", "code": "timeout", "message": message}
        meta = {"id": "run2", "status": "error", "error": error}
        result = {"structuredContent": {"error": error}, "isError": True}
        result["_meta"] = {"com.browseros.neo/run": meta}
        self.capture(
            "c2",
            P.ChannelId.INPUT_VALUE,
            json.dumps({"jsonrpc": "2.0", "id": 2, "result": result}).encode(),
            run_id="run2",
            proofs=("p1", "p1"),
            regions=(asked.id,),
        )
        traceback = "Traceback: HarnessError: takeover refused: 409 " + "\n".join(forms)
        failed = J.Step(9, "takeover", self.inst.name, "failed", {"error": traceback})
        card = self.inst.plant(P.Kind.CC_NUMBER, P.Location.MAIN).secret.reveal()
        facts = J.Facts("img", None, None, {}, {"check": f"FAIL {card}"}, otp, 0, {})
        self.records += [J.encode(failed), J.encode(facts)]
        evidence = self.evidence()
        judgement = J.judge(evidence)
        report = J.render_markdown(judgement, evidence)
        verdicts = J.render_json(judgement, evidence)
        self.assertIn(
            "refused: neo/timeout", verdicts, "the refusal's message is shown"
        )
        self.assertIn("takeover refused", report, "the failure is shown")
        detector = J.Detector([self.inst])
        for name, views in (
            ("report.md", J.views_of(report.encode(), structured=False)),
            ("verdicts.json", J.views_of(verdicts.encode())),
        ):
            with self.subTest(output=name):
                self.assertEqual(detector.hits(views.raw), [])
                raw = [
                    r.id
                    for r in self.inst.regions
                    if r.short and detector.slot_state(views, r) is J.SlotState.RAW
                ]
                self.assertEqual(raw, [])
                echoed = [f"{{{{{r.expected()}}}}}" for r in self.inst.regions]
                self.assertEqual([e for e in echoed if e in views.raw], [])

    def test_a_record_inherits_presence_from_its_own_run(self) -> None:
        (field,) = self.inst.field_regions(self.inst.plant(P.Kind.OTP, P.Location.MAIN))
        self.proof("p1", [])
        self.proof("p2", [field])
        self.capture(
            "c1",
            P.ChannelId.INPUT_VALUE,
            self.reply({field.id: {"ok": ""}}, "run1"),
            run_id="run1",
            proofs=("p1", "p1"),
            regions=(field.id,),
        )
        self.capture(
            "c2",
            P.ChannelId.INPUT_VALUE,
            self.reply({field.id: {"ok": "••••"}}, "run2"),
            run_id="run2",
            proofs=("p2", "p2"),
            regions=(field.id,),
        )
        trace = [
            {"runId": "run1", "result": {field.id: {"ok": ""}}},
            {"runId": "run2", "result": {field.id: {"ok": "••••"}}},
        ]
        self.capture(
            "h1", P.ChannelId.TRACE, json.dumps(trace).encode(), phase=P.Phase.ANY
        )
        cells = [c for c in self.judged() if c.channel is P.ChannelId.TRACE]
        self.assertEqual(
            [(c.run_id, c.verdict) for c in cells], [("run2", J.Verdict.MASKED)]
        )


def matcher_error(locator: str, received: str, polled: str | None = None) -> dict:
    """An expect failure as a keyed error, its message as browseros-playwright prints it
    (expect/matcher_message.rs): Received escapes `"` and `\\`, a poll line does not."""
    quoted = received.replace("\\", "\\\\").replace('"', '\\"')
    polled = received if polled is None else polled
    message = (
        "expect(locator).toHaveValue(expected) failed\n\n"
        f'Locator:  {locator}\nExpected: "••••"\nReceived: "{quoted}"\n'
        "Timeout:  800ms\n\nCall log:\n"
        f'  - Expect "toHaveValue" {locator} with timeout 800ms\n'
        f"  - waiting for {locator}\n"
        f'    3 × locator resolved to <input type="text" id="x"/>\n'
        f'       - unexpected value "{polled}"\n'
    )
    return {"err": message, "owner": "website", "code": "assertion_failed"}


class KeysTests(EvidenceBase):
    """The keys scenario's rows through judge(): expect messages, action errors, copy
    probes, the agent's typing and its records, and the clipboard on a view display."""

    layout = P.Layout.FIELDS

    def field(self, kind: P.Kind, loc: P.Location = P.Location.MAIN) -> P.Region:
        return self.inst.field_regions(self.inst.plant(kind, loc))[0]

    def verdicts(self, channel: P.ChannelId) -> dict[str, J.Cell]:
        return {c.region.id: c for c in self.judged() if c.channel is channel}

    def test_an_expect_failure_is_judged_by_its_received_line(self) -> None:
        """The Received line (and each poll's) is the region's slot in its own keyed
        error, so a masked Received is masked, a raw one leaks (a 3-digit one included),
        and an error with no Received line shows nothing of the field."""
        masked, cvc, otp, plain = (
            self.field(k)
            for k in (P.Kind.PASSWORD, P.Kind.CC_CSC, P.Kind.OTP, P.Kind.PLAIN)
        )
        polled = self.field(P.Kind.OTP_SPLIT)  # one digit: short
        bare = self.field(P.Kind.WORD_OTP)
        regions = [masked, cvc, otp, plain, polled, bare]
        value = {
            r.id: matcher_error(f"locator('#{r.id}')", r.expected())
            for r in (cvc, otp, plain)
        }
        value[masked.id] = matcher_error(f"locator('#{masked.id}')", "••••")
        value[polled.id] = matcher_error(
            f"locator('#{polled.id}')", "••••", polled.expected()
        )
        value[bare.id] = {
            "err": "Timeout 800ms exceeded.",
            "owner": "website",
            "code": "assertion_failed",
        }
        self.proof("p1", regions)
        self.capture(
            "c1",
            P.ChannelId.TO_HAVE_VALUE,
            self.reply(value, "run1"),
            run_id="run1",
            proofs=("p1", "p1"),
            regions=tuple(r.id for r in regions),
            phase=P.Phase.ANY,
        )
        cells = self.verdicts(P.ChannelId.TO_HAVE_VALUE)
        got = {r.id: (cells[r.id].verdict, cells[r.id].residual) for r in regions}
        self.assertEqual(
            got,
            {
                masked.id: (J.Verdict.MASKED, None),
                cvc.id: (J.Verdict.LEAKED, None),
                otp.id: (J.Verdict.LEAKED, None),
                plain.id: (J.Verdict.LEAKED, "CONTROL"),
                polled.id: (J.Verdict.LEAKED, None),
                bare.id: (J.Verdict.INCONCLUSIVE, None),
            },
        )

    def test_an_actions_strict_preview_is_judged_by_its_value_attributes(self) -> None:
        """A strict-mode error previews value attributes. A preview with a masked value
        covers its field even when its `aka` names no anchor; a field it does not list is
        n/a; a raw one leaks, three digits included."""
        self.inst = P.make_instance(P.Layout.LOCKED, seed="ev", index=1)
        pw, cvc, otp, unlisted = (
            self.field(k)
            for k in (P.Kind.PASSWORD, P.Kind.CC_CSC, P.Kind.OTP, P.Kind.WORD_OTP)
        )
        lines = [
            f'<input readonly id="{pw.id}" type="password" value="••••"/> '
            f"aka getByLabel('Field {pw.anchor}')",
            f'<input readonly id="{otp.id}" value="••••"/> aka locator(\'#{otp.id}\')',
            f'<input readonly id="{cvc.id}" value="{cvc.expected()}"/> '
            f"aka locator('#{cvc.id}')",
        ]
        message = "strict mode violation: locator('input') resolved to 36 elements:"
        message += "".join(f"\n    {n}) {line}" for n, line in enumerate(lines, 1))
        error = {"owner": "script", "code": "strict_mode_violation", "message": message}
        regions = [pw, cvc, otp, unlisted]
        self.proof("p1", regions)
        self.capture(
            "c1",
            P.ChannelId.FILL_STRICT,
            self.reply(None, "run1", error),
            run_id="run1",
            proofs=("p1", "p1"),
            regions=tuple(r.id for r in regions),
            phase=P.Phase.ANY,
        )
        cells = self.verdicts(P.ChannelId.FILL_STRICT)
        self.assertEqual(
            {r.id: cells[r.id].verdict for r in regions},
            {
                pw.id: J.Verdict.MASKED,
                otp.id: J.Verdict.MASKED,
                cvc.id: J.Verdict.LEAKED,
                unlisted.id: J.Verdict.NOT_APPLICABLE,
            },
        )

    def test_a_fill_call_log_reaches_no_field_unless_it_previews_one(self) -> None:
        """fill's call log names its locator and the states it waited for. By contract
        it previews no element, so a field is n/a there; a preview that does appear is
        judged like any slot."""
        self.inst = P.make_instance(P.Layout.LOCKED, seed="ev", index=1)
        quiet, shown = self.field(P.Kind.PASSWORD), self.field(P.Kind.CC_CSC)
        log = (
            "Timeout 800ms exceeded.\nCall log:\n  - waiting for locator('#{id}')\n"
            "  - attempting fill action\n"
            "    - waiting for element to be visible, enabled and editable\n"
            "    - element is not editable\n{extra}"
        )
        preview = f'  - locator resolved to <input readonly id="{shown.id}" '
        preview += f'value="{shown.expected()}"/>\n'
        value = {
            r.id: {
                "err": log.format(id=r.id, extra=extra),
                "owner": "website",
                "code": "not_actionable",
            }
            for r, extra in ((quiet, ""), (shown, preview))
        }
        self.proof("p1", [quiet, shown])
        self.capture(
            "c1",
            P.ChannelId.FILL_CALL_LOG,
            self.reply(value, "run1"),
            run_id="run1",
            proofs=("p1", "p1"),
            regions=(quiet.id, shown.id),
            phase=P.Phase.ANY,
        )
        cells = self.verdicts(P.ChannelId.FILL_CALL_LOG)
        self.assertEqual(cells[quiet.id].verdict, J.Verdict.NOT_APPLICABLE)
        self.assertEqual(cells[shown.id].verdict, J.Verdict.LEAKED)

    def copy_probe(
        self,
        n: int,
        field: P.Region,
        *,
        chord: dict,
        pasted: str,
        token: str,
        channel: P.ChannelId = P.ChannelId.COPY_CONTROL_C,
        control_error: dict | None = None,
        present: bool = True,
        focused: bool = True,
    ) -> None:
        """One probe as the Session records it: the baseline copy, a proof, the chord
        and the paste in one run, a proof with the paste box's digest."""
        self.capture(
            f"a{n}",
            P.ChannelId.HARNESS,
            self.reply(1, f"base{n}", control_error),
            run_id=f"base{n}",
            phase=P.Phase.ANY,
        )
        focus = (field.id,) if focused else (P.COPY_BASE,)
        self.proof(f"p{n}b", [field] if present else [], {P.COPY_BASE: token}, focus)
        self.capture(
            f"c{n}",
            channel,
            self.reply({"chord": chord, "paste": {"sent": True}}, f"run{n}"),
            run_id=f"run{n}",
            proofs=(f"p{n}b", f"p{n}a"),
            regions=(field.id,),
            control=f"a{n}",
            phase=P.Phase.ANY,
        )
        self.proof(f"p{n}a", [field], {P.COPY_BASE: token, P.COPY_DROP: pasted})

    def test_a_copy_probe_is_judged_by_what_the_paste_box_holds(self) -> None:
        """The field's value pasted back is laundered, whatever the chord answered;
        held and masked need a sent baseline copy whose fresh token came back."""
        self.inst = P.make_instance(P.Layout.COPY, seed="ev", index=1)
        refused = {
            "err": "copying or cutting from a password, card or one-time-code field "
            "is refused; read what the page shows instead",
            "owner": "script",
            "code": "sensitive_copy_refused",
        }
        sent = {"sent": True}
        kinds = (
            P.Kind.PASSWORD,
            P.Kind.CC_NUMBER,
            P.Kind.CC_CSC,
            P.Kind.OTP,
            P.Kind.WORD_CARD,
            P.Kind.WORD_OTP,
            P.Kind.NEW_PASSWORD,
            P.Kind.CURRENT_PASSWORD,
            P.Kind.PLAIN,
        )
        f = {kind: self.field(kind, P.Location.SAME_SITE) for kind in kinds}
        lied = f[P.Kind.CC_NUMBER].expected()
        self.copy_probe(1, f[P.Kind.PASSWORD], chord=refused, pasted="t1", token="t1")
        self.copy_probe(2, f[P.Kind.CC_CSC], chord=sent, pasted="t2", token="t2")
        self.copy_probe(3, f[P.Kind.CC_NUMBER], chord=refused, pasted=lied, token="t3")
        self.copy_probe(4, f[P.Kind.OTP], chord=sent, pasted="", token="t4")
        self.copy_probe(
            5,
            f[P.Kind.WORD_CARD],
            chord=refused,
            pasted="t5",
            token="t5",
            control_error=refused,
        )
        self.copy_probe(6, f[P.Kind.WORD_OTP], chord=refused, pasted="t5", token="t5")
        self.copy_probe(
            7,
            f[P.Kind.NEW_PASSWORD],
            chord=refused,
            pasted="t7",
            token="t7",
            present=False,
        )
        plain = f[P.Kind.PLAIN]
        self.copy_probe(8, plain, chord=sent, pasted=plain.expected(), token="t8")
        # The page's focus missed: the chord copied the selected token again.
        unfocused = f[P.Kind.CURRENT_PASSWORD]
        self.copy_probe(
            9, unfocused, chord=sent, pasted="t9", token="t9", focused=False
        )
        cells = self.verdicts(P.ChannelId.COPY_CONTROL_C)
        got = {k: (cells[r.id].verdict, cells[r.id].residual) for k, r in f.items()}
        self.assertEqual(
            got,
            {
                P.Kind.PASSWORD: (J.Verdict.WITHHELD, None),
                P.Kind.CC_CSC: (J.Verdict.MASKED, None),
                P.Kind.CC_NUMBER: (J.Verdict.LEAKED, None),
                P.Kind.OTP: (J.Verdict.INCONCLUSIVE, None),
                P.Kind.WORD_CARD: (J.Verdict.INCONCLUSIVE, None),
                P.Kind.WORD_OTP: (J.Verdict.INCONCLUSIVE, None),
                P.Kind.NEW_PASSWORD: (J.Verdict.INCONCLUSIVE, None),
                P.Kind.CURRENT_PASSWORD: (J.Verdict.INCONCLUSIVE, None),
                P.Kind.PLAIN: (J.Verdict.LEAKED, "CONTROL"),
            },
        )
        self.assertIn("laundered", cells[f[P.Kind.CC_NUMBER].id].why)
        self.assertIn("focus", cells[unfocused.id].why)
        self.assertIn("not new", cells[f[P.Kind.WORD_OTP].id].why)

    def test_a_view_display_judges_each_copy_against_its_window(self) -> None:
        """Any ServerCutText is a leak of the copy it follows, and of no other: one
        arriving during the second probe marks that probe's field only. A short field
        has no slot here, and needs none: no cut text at all proves its digits absent."""
        self.inst = P.make_instance(P.Layout.COPY, seed="ev", index=1)
        first, second = self.field(P.Kind.CC_NUMBER), self.field(P.Kind.OTP)
        short = self.field(P.Kind.CC_CSC)
        refused = {
            "err": "refused",
            "owner": "script",
            "code": "sensitive_copy_refused",
        }
        probes = ((short, 60), (first, 100), (second, 200))
        for n, (field, since) in enumerate(probes, 1):
            self.copy_probe(n, field, chord=refused, pasted=f"t{n}", token=f"t{n}")
            self.step(
                since + 50,
                "copy",
                region=field.id,
                capture=f"c{n}",
                since_ms=since,
                phase="any",
            )
        pixel_format = bytes([32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0])
        cut = b"\x03\x00\x00\x00" + struct.pack(">i", 4) + b"zzzz"
        self.stream(
            "s1",
            P.ChannelId.DISPLAY_VIEW,
            cut,
            instance=self.inst.name,
            target="/v1/display?mode=view",
            status=101,
            role="view",
            geometry=(4, 4, pixel_format.hex()),
            opened_ms=50,
            chunks=((0, 250),),
            closed=(400, "harness"),
        )
        cells = self.verdicts(P.ChannelId.CLIPBOARD_VIEW)
        self.assertEqual(
            {r.id: cells[r.id].verdict for r in (short, first, second)},
            {
                short.id: J.Verdict.MASKED,
                first.id: J.Verdict.MASKED,
                second.id: J.Verdict.LEAKED,
            },
        )

    def typed(self, n: int, region: P.Region, record: dict) -> None:
        """One typing run as the Session records it, and its run record."""
        reachable = P.PLACES[region.location].agent_reachable
        value = {region.id: {"ok": "••••"}} if reachable else {}
        self.proof(f"t{n}b", [])
        self.proof(f"t{n}a", [region])
        self.capture(
            f"t{n}",
            P.ChannelId.KEYBOARD_TYPE,
            self.reply(value, f"run{n}"),
            run_id=f"run{n}",
            proofs=(f"t{n}b", f"t{n}a"),
            regions=(region.id,),
            phase=P.Phase.ANY,
            request=P.typing_code(self.inst, region),
        )
        self.capture(
            f"h{n}",
            P.ChannelId.RUN,
            json.dumps({"id": f"run{n}", **record}).encode(),
            run_id=f"run{n}",
            phase=P.Phase.ANY,
        )

    def test_a_typing_runs_records_are_judged_by_what_they_kept(self) -> None:
        """A typing run's record keeps what it typed in two places: the op's typed
        argument and the code's literal. Masked or withheld there is masked (the closed
        root's record names no region at all); a raw digit there is a leak; the PLAIN
        control's code is stored as sent, which the search must find."""
        self.inst = P.make_instance(P.Layout.TYPED, seed="ev", index=1)
        pw = self.field(P.Kind.PASSWORD)
        box = self.field(P.Kind.OTP_SPLIT)
        closed = self.field(P.Kind.CC_CSC, P.Location.SHADOW_CLOSED)
        plain = self.field(P.Kind.PLAIN)

        def ops(run: str, region: P.Region, text: str) -> list[dict]:
            focus = {"name": "locator.focus", "args": {"selector": f"#{region.id}"}}
            typing = {"name": "keyboard.type", "args": {"text": text}}
            if not P.PLACES[region.location].agent_reachable:
                focus = {"name": "page.title", "args": {}}
            return [
                {"id": f"{run}.{n}", "runId": run, "startedAt": 0, **op}
                for n, op in enumerate((focus, typing), 1)
            ]

        withheld = {"code": None, "codeWithheld": "sensitive_input"}
        self.typed(1, pw, {**withheld, "operations": ops("run1", pw, "[redacted]")})
        self.typed(2, box, {**withheld, "operations": ops("run2", box, box.expected())})
        self.typed(
            3, closed, {**withheld, "operations": ops("run3", closed, "[redacted]")}
        )
        code = P.typing_code(self.inst, plain)
        self.typed(
            4, plain, {"code": code, "operations": ops("run4", plain, plain.expected())}
        )
        cells = self.verdicts(P.ChannelId.RUN)
        got = {
            r.id: (cells[r.id].verdict, cells[r.id].residual)
            for r in (pw, box, closed, plain)
        }
        self.assertEqual(
            got,
            {
                pw.id: (J.Verdict.MASKED, None),
                box.id: (J.Verdict.LEAKED, None),
                closed.id: (J.Verdict.MASKED, None),
                plain.id: (J.Verdict.LEAKED, "CONTROL"),
            },
        )

    def test_the_report_never_prints_a_value_the_new_rows_carry(self) -> None:
        """A leak block prints its request: here a typing run's code, which carries an
        agent value (a long SPECIAL one and one split digit), and expect messages whose
        Received and poll lines hold raw values. report.md and verdicts.json, read back
        as answers, hold none of them."""
        self.inst = P.make_instance(P.Layout.TYPED, seed="ev", index=1)
        pw, box = self.field(P.Kind.PASSWORD), self.field(P.Kind.OTP_SPLIT)
        cvc = self.field(P.Kind.CC_CSC)
        for n, region in enumerate((pw, box), 1):
            self.proof(f"t{n}b", [])
            self.proof(f"t{n}a", [region, cvc])
            leaked = {region.id: {"ok": region.expected()}}  # the read-back, raw
            leaked[cvc.id] = matcher_error(f"locator('#{cvc.id}')", cvc.expected())
            self.capture(
                f"t{n}",
                P.ChannelId.KEYBOARD_TYPE,
                self.reply(leaked, f"run{n}"),
                run_id=f"run{n}",
                proofs=(f"t{n}b", f"t{n}a"),
                regions=(region.id,),
                phase=P.Phase.ANY,
                request=json.dumps({"code": P.typing_code(self.inst, region)}),
            )
        evidence = self.evidence()
        judgement = J.judge(evidence)
        report = J.render_markdown(judgement, evidence)
        verdicts = J.render_json(judgement, evidence)
        self.assertIn("keyboard.type", report, "the typing run's leak is reported")
        self.assertIn("request:", report, "with its request")
        detector = J.Detector([self.inst])
        for name, text in (("report.md", report), ("verdicts.json", verdicts)):
            views = J.views_of(text.encode(), structured=name.endswith(".json"))
            with self.subTest(output=name):
                self.assertEqual(detector.hits(views.raw), [])
                raw = [
                    r.id
                    for r in self.inst.regions
                    if r.short and detector.slot_state(views, r) is J.SlotState.RAW
                ]
                self.assertEqual(raw, [])
                for region in (box, cvc):
                    value = re.escape(region.expected())
                    for form in (
                        rf'type\(\\*"{value}\\*"\)',
                        rf'Received: \\*"{value}',
                        rf'unexpected value \\*"{value}',
                    ):
                        self.assertIsNone(re.search(form, text), form)


MASK = "[private]"


class ScriptTests(EvidenceBase):
    """Main-world script (the evaluate family) through judge(): the grouped answer the
    probes' readers return, every spelling of a region's text, held answers, and the
    echo of a cross-site value that no sweep of the page's site sees."""

    layout = P.Layout.SCRIPT

    def field(self, kind: P.Kind, loc: P.Location = P.Location.MAIN) -> P.Region:
        return self.inst.field_regions(self.inst.plant(kind, loc))[0]

    def echo(self, loc: P.Location, kind: P.Kind = P.Kind.PASSWORD) -> P.Region:
        return echo_of(self.inst, self.inst.plant(kind, loc))

    @staticmethod
    def grouped(reads: dict[P.Region, dict[str, str]]) -> dict:
        """What the readers return: {spelling: {region id: {ok}}}, with `near` (the
        field's label, which carries its anchor) beside each plain read of a field."""
        out: dict = {"plain": {}, "json": {}, "uri": {}}
        for region, spelled in reads.items():
            for spelling, text in spelled.items():
                out[spelling][region.id] = {"ok": text}
            if region.exposure is P.Exposure.FIELD:
                out["plain"][region.id]["near"] = f"Field {region.anchor} "
        return out

    def probe(self, channel: P.ChannelId, value: object, regions, **kw) -> None:
        self.proof("p1", list(regions))
        self.capture(
            "c1",
            channel,
            self.reply(value, "run1", kw.pop("error", None)),
            run_id="run1",
            proofs=("p1", "p1"),
            regions=tuple(r.id for r in kw.pop("asked", regions)),
            **kw,
        )

    def verdicts(self, channel: P.ChannelId) -> dict[str, tuple]:
        return {
            c.region.id: (c.verdict, c.residual)
            for c in self.judged()
            if c.channel is channel
        }

    def test_a_main_world_read_is_judged_in_every_spelling_it_returns(self) -> None:
        """A masked spelling is masked, a JSON string of the mask too; one raw spelling
        leaks its region; the PLAIN control reads back raw; an echo is read in its
        slot whatever spelling wraps it."""
        pw, so = (
            self.field(P.Kind.PASSWORD),
            self.field(P.Kind.PASSWORD, P.Location.SAME_ORIGIN),
        )
        cvc, box = self.field(P.Kind.CC_CSC), self.field(P.Kind.OTP_SPLIT)
        otp, plain = self.field(P.Kind.OTP), self.field(P.Kind.PLAIN)
        echo = self.echo(P.Location.MAIN, P.Kind.WORD_PASSWORD)
        masked = {"plain": MASK, "json": json.dumps(MASK), "uri": MASK}
        echoed = f"ECHO {echo.anchor} {{{{{MASK}}}}}"
        uri_echo = urllib.parse.quote(f"ECHO {echo.anchor} {{{{") + MASK
        reads = {
            pw: masked,
            so: masked,
            cvc: {"plain": MASK},
            box: {"plain": MASK},
            otp: {**masked, "json": json.dumps(otp.expected())},
            plain: {
                "plain": plain.expected(),
                "json": json.dumps(plain.expected()),
                "uri": plain.expected(),
            },
            echo: {
                "plain": echoed,
                "json": json.dumps(echoed),
                "uri": uri_echo + urllib.parse.quote("}}"),
            },
        }
        self.probe(P.ChannelId.EVALUATE, self.grouped(reads), reads, phase=P.Phase.COLD)
        got = self.verdicts(P.ChannelId.EVALUATE)
        masked_cell = (J.Verdict.MASKED, None)
        self.assertEqual(
            {r.id: got[r.id] for r in reads},
            {
                pw.id: masked_cell,
                so.id: masked_cell,
                cvc.id: masked_cell,
                box.id: masked_cell,
                otp.id: (J.Verdict.LEAKED, None),
                plain.id: (J.Verdict.LEAKED, "CONTROL"),
                echo.id: masked_cell,
            },
        )

    def test_a_field_read_is_covered_by_the_anchor_its_label_carries(self) -> None:
        """The anchor beside the value proves the reader read that field, whatever the
        run's code named: a near label alone covers it; a read with neither keyed ok
        nor anchor proves nothing."""
        pw, otp = self.field(P.Kind.PASSWORD), self.field(P.Kind.OTP)
        value = {"plain": {pw.id: {"near": f"Field {pw.anchor} "}}, "masked": MASK}
        self.probe(P.ChannelId.EVALUATE, value, [pw, otp], phase=P.Phase.COLD)
        got = self.verdicts(P.ChannelId.EVALUATE)
        self.assertEqual(got[otp.id][0], J.Verdict.INCONCLUSIVE)
        self.assertEqual(got[pw.id], (J.Verdict.MASKED, None))

    def test_held_main_world_script_is_held_where_it_reaches(self) -> None:
        """Today every main-world op answers script/method_unavailable: held (EVAL-HELD)
        for each region the probe asked, n/a for what it cannot reach."""
        pw, echo = self.field(P.Kind.PASSWORD), self.echo(P.Location.SAME_SITE)
        ss = self.field(P.Kind.PASSWORD, P.Location.SAME_SITE)
        error = {
            "owner": "script",
            "code": "method_unavailable",
            "message": "page.evaluate is not available for now",
        }
        self.probe(
            P.ChannelId.EVALUATE,
            None,
            [pw, echo, ss],
            error=error,
            asked=[pw, echo],
            phase=P.Phase.COLD,
        )
        got = self.verdicts(P.ChannelId.EVALUATE)
        held = (J.Verdict.WITHHELD, "EVAL-HELD")
        self.assertEqual(got[pw.id], held)
        self.assertEqual(got[echo.id], held)
        self.assertEqual(got[ss.id], (J.Verdict.NOT_APPLICABLE, None))

    def test_a_cross_site_echo_raw_in_main_world_script_is_xsite(self) -> None:
        """The guard sweeps the page's site, so a b.test value the page copied into the
        top document reads raw cold: by design (XSITE), as for a text read. A same-site
        echo read raw is the guard missing a frame: a new leak."""
        xo, ss = self.echo(P.Location.CROSS_SITE), self.echo(P.Location.SAME_SITE)
        reads = {
            r: {"plain": f"ECHO {r.anchor} {{{{{r.expected()}}}}}"} for r in (xo, ss)
        }
        self.probe(P.ChannelId.EVALUATE, self.grouped(reads), reads, phase=P.Phase.COLD)
        got = self.verdicts(P.ChannelId.EVALUATE)
        self.assertEqual(got[xo.id], (J.Verdict.LEAKED, "XSITE"))
        self.assertEqual(got[ss.id], (J.Verdict.LEAKED, None))

    def test_the_headline_counts_what_served_main_world_script_returned(self) -> None:
        """Held, the headline counts held channels; served, it counts the reads of
        sensitive values that came back masked and raw. The control is not counted."""
        pw, otp = self.field(P.Kind.PASSWORD), self.field(P.Kind.OTP)
        plain = self.field(P.Kind.PLAIN)
        reads = {
            pw: {"plain": MASK},
            otp: {"plain": otp.expected()},
            plain: {"plain": plain.expected()},
        }
        self.probe(P.ChannelId.EVALUATE, self.grouped(reads), reads, phase=P.Phase.COLD)
        headline = J.judge(self.evidence()).headline["main-world script"]
        self.assertEqual(headline, "served: raw in 1 of 2 reads (1 new)")

    def test_an_element_scripts_answer_holds_one_outcome_per_element(self) -> None:
        """locator.evaluate and its relatives answer each element on its own: a held op
        is held there, an op not built is n/a, a masked one masked, a raw short value a
        leak (the guard masks a whole short string)."""
        pw, cur = self.field(P.Kind.PASSWORD), self.field(P.Kind.CURRENT_PASSWORD)
        otp, cvc = self.field(P.Kind.OTP), self.field(P.Kind.CC_CSC)
        held = {"err": "held", "owner": "script", "code": "method_unavailable"}
        missing = {"err": "no handles", "owner": "neo", "code": "internal_error"}
        value = self.grouped(
            {otp: {"plain": MASK, "json": json.dumps(MASK), "uri": MASK}}
        )
        value["plain"].update({pw.id: held, cur.id: missing})
        value["plain"][cvc.id] = {"ok": cvc.expected(), "near": f"Field {cvc.anchor} "}
        self.probe(P.ChannelId.LOCATOR_EVALUATE, value, [pw, cur, otp, cvc])
        got = self.verdicts(P.ChannelId.LOCATOR_EVALUATE)
        self.assertEqual(
            {r.id: got[r.id] for r in (pw, cur, otp, cvc)},
            {
                pw.id: (J.Verdict.WITHHELD, "EVAL-HELD"),
                cur.id: (J.Verdict.NOT_APPLICABLE, None),
                otp.id: (J.Verdict.MASKED, None),
                cvc.id: (J.Verdict.LEAKED, None),
            },
        )


class ContentTests(EvidenceBase):
    """page.setContent through judge(): the op itself answers nothing, so its row says
    only whether Neo ran it; the reads of what it wrote are judged as any read."""

    layout = P.Layout.CONTENT

    def regions(self, exposure: P.Exposure) -> list[P.Region]:
        return [r for r in self.inst.regions if r.exposure is exposure]

    def test_an_op_that_answers_nothing_says_only_whether_neo_ran_it(self) -> None:
        """Before the op the shell shows nothing, so its cells stand on the regions it
        would write: held by contract (EVAL-HELD), not built, or n/a once it ran. A
        value in its answer is still a leak."""
        echoes = self.regions(P.Exposure.ECHO)
        self.proof("p0", [])
        outcomes = {
            "c1": {"error": {"owner": "script", "code": "method_unavailable"}},
            "c2": {"error": {"owner": "neo", "code": "unsupported"}},
            "c3": {"value": 1},
            "c4": {"value": {"echo": echoes[0].plant.secret.reveal()}},
        }
        for ident, outcome in outcomes.items():
            error = outcome.get("error")
            if error:
                error = {**error, "message": "setContent is not available for now"}
            self.capture(
                ident,
                P.ChannelId.SET_CONTENT,
                self.reply(outcome.get("value"), f"run{ident}", error),
                run_id=f"run{ident}",
                proofs=("p0", "p0"),
                regions=tuple(r.id for r in echoes),
                phase=P.Phase.ANY,
            )
        cells = [c for c in self.judged() if c.channel is P.ChannelId.SET_CONTENT]
        by_source: dict[str, set] = {}
        for c in cells:
            if c.region.id in {r.id for r in echoes}:
                by_source.setdefault(c.source, set()).add((c.verdict, c.residual))
        self.assertEqual(by_source["c1"], {(J.Verdict.WITHHELD, "EVAL-HELD")})
        headline = J.judge(self.evidence()).headline
        self.assertEqual(
            headline["page.setContent, then reads of what it wrote"],
            "held (script/method_unavailable)",
        )
        self.assertEqual(by_source["c2"], {(J.Verdict.NOT_APPLICABLE, None)})
        self.assertEqual(by_source["c3"], {(J.Verdict.NOT_APPLICABLE, None)})
        leaked = {(c.verdict, c.residual) for c in cells if c.source == "c4"}
        self.assertIn((J.Verdict.LEAKED, None), leaked)
        why = {c.why for c in cells if c.source == "c2"}
        self.assertEqual(why, {"not built (neo/unsupported)"})

    def test_a_read_of_the_written_page_is_judged_as_any_read(self) -> None:
        """After setContent the title, content and text are ordinary reads: a masked
        echo is masked, a raw one is a new leak (its sweep should have caught it), a raw
        CVC is by design (BD-SHORT)."""
        long_titles = [r for r in self.regions(P.Exposure.TITLE) if not r.short]
        short = next(r for r in self.regions(P.Exposure.TITLE) if r.short)
        masked, raw = long_titles[0], long_titles[1]
        title = " ".join(
            f"ECHO {r.anchor} {{{{{text}}}}}"
            for r, text in (
                (masked, MASK),
                (raw, raw.expected()),
                (short, short.expected()),
            )
        )
        self.proof("p1", [masked, raw, short])
        self.capture(
            "c1",
            P.ChannelId.TITLE,
            self.reply(title, "run1"),
            run_id="run1",
            proofs=("p1", "p1"),
            regions=tuple(r.id for r in (masked, raw, short)),
            phase=P.Phase.AFTER_CONTENT,
        )
        got = {
            c.region.id: (c.verdict, c.residual)
            for c in self.judged()
            if c.channel is P.ChannelId.TITLE
        }
        self.assertEqual(got[masked.id], (J.Verdict.MASKED, None))
        self.assertEqual(got[raw.id], (J.Verdict.LEAKED, None))
        self.assertEqual(got[short.id], (J.Verdict.LEAKED, "BD-SHORT"))
        headline = J.judge(self.evidence()).headline
        self.assertEqual(
            headline["page.setContent, then reads of what it wrote"],
            "raw in 1 of 2 reads",
        )


# One RFB FramebufferUpdate: one 1x1 Raw rect at 32 bits per pixel.
UPDATE = b"\x00\x00\x00\x01" + struct.pack(">HHHHi", 0, 0, 1, 1, 0) + b"\x00" * 4
RFB_FORMAT = bytes([32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]).hex()


class FrontTests(EvidenceBase):
    """page.bringToFront through judge(): view displays against the moment a private
    tab came to the front, the op's own row, and the headline."""

    layout = P.Layout.PIXELS

    def fields(self) -> list[P.Region]:
        return [r for r in self.inst.regions if r.exposure is P.Exposure.FIELD]

    def bring(self, ident: str, value: object) -> None:
        """The agent's bringToFront on this page, its fields proven around it (what the
        view cells of the page stand on)."""
        self.proof(f"{ident}p", self.fields())
        self.capture(
            ident,
            P.ChannelId.BRING_TO_FRONT,
            self.reply(value, ident),
            run_id=ident,
            proofs=(f"{ident}p", f"{ident}p"),
            regions=tuple(r.id for r in self.fields()),
            phase=P.Phase.ANY,
        )

    def front_step(self, at: int, **detail: object) -> None:
        detail = {"answer": "sent", "private": True, "in_front": True, **detail}
        self.records.append(
            J.encode(J.Step(at, "front", self.inst.name, "front", detail))
        )

    def view(self, ident: str, arrivals: list[int], **kw) -> None:
        self.stream(
            ident,
            P.ChannelId.FRONT_VIEW,
            UPDATE * len(arrivals),
            instance=self.inst.name,
            target="/v1/display?mode=view",
            status=kw.pop("status", 101),
            role=kw.pop("role", "view"),
            geometry=(4, 4, RFB_FORMAT),
            opened_ms=0,
            chunks=tuple((n * len(UPDATE), at) for n, at in enumerate(arrivals)),
            closed=kw.pop("closed", (9000, "close 4001")),
        )

    def views(self) -> dict[str, set]:
        out: dict[str, set] = {}
        for c in self.judged():
            if c.channel is P.ChannelId.FRONT_VIEW:
                out.setdefault(c.source, set()).add((c.verdict, c.why))
        return out

    def test_a_view_still_sending_after_a_private_tab_came_forward_leaks(self) -> None:
        """No view while a private tab is in front (decision 34), and every view closes
        before the switch (decision 46): a frame later than the transport allowance after
        it leaks, whatever it shows, though a screencast's pixel grace would excuse it;
        frames before it, or in flight at it, and none later are masked; a view refused
        once the tab is in front is held."""
        self.bring("b1", {"sent": True})
        self.front_step(1000, since_ms=1000)
        late = 1000 + J.FRONT_GRACE_MS + 100
        self.assertLess(late, 1000 + J.PIXEL_GRACE_MS)
        self.view("s1", [200, late])
        self.view(
            "s2", [200, 1000 + J.FRONT_GRACE_MS // 2], closed=(1300, "close 4001")
        )
        self.view("s3", [], status=409, role="retry", closed=(7000, "http 409"))
        got = self.views()
        self.assertEqual({v for v, _ in got["s1"]}, {J.Verdict.LEAKED})
        self.assertEqual({v for v, _ in got["s2"]}, {J.Verdict.MASKED})
        self.assertEqual({v for v, _ in got["s3"]}, {J.Verdict.WITHHELD})

    def test_views_are_na_while_no_private_tab_came_forward(self) -> None:
        """Refused or built nowhere, bringToFront never put the private tab in front:
        the views had nothing to keep from anyone."""
        self.bring(
            "b1",
            {
                r.id: {"err": "x", "owner": "neo", "code": "unsupported"}
                for r in self.fields()
            },
        )
        self.front_step(1000, answer="neo/unsupported", in_front=False, since_ms=1000)
        self.view("s1", [200, 1500, 9000])
        self.assertEqual(
            self.views()["s1"],
            {(J.Verdict.NOT_APPLICABLE, "the private tab never came to the front")},
        )

    def test_the_ops_row_is_held_when_the_owner_has_control(self) -> None:
        refusal = {"err": "x", "owner": "host", "code": "owner_has_control"}
        self.bring("b1", {r.id: refusal for r in self.fields()})
        self.bring("b2", {"sent": True})
        got = {
            (c.source, c.verdict)
            for c in self.judged()
            if c.channel is P.ChannelId.BRING_TO_FRONT
        }
        self.assertEqual(
            got,
            {("b1", J.Verdict.WITHHELD), ("b2", J.Verdict.NOT_APPLICABLE)},
        )

    def test_the_headline_says_what_bring_to_front_did(self) -> None:
        self.front_step(
            1000, since_ms=1000, before=["i01-pixels"], after=["i01-pixels"]
        )
        self.front_step(
            2000,
            answer="host/owner_has_control",
            private=False,
            in_front=False,
            during="i09-pixels",
            before=["i09-pixels"],
            after=["i09-pixels"],
        )
        headline = J.judge(self.evidence()).headline
        self.assertEqual(
            headline["page.bringToFront onto a private tab"],
            "sent; the private tab came to the front",
        )
        self.assertEqual(
            headline["page.bringToFront during a takeover"],
            "host/owner_has_control; the takeover's tab stayed in front",
        )


class UploadTests(EvidenceBase):
    """setInputFiles through judge(): the agent reads back a file Neo never saw (by
    design), its own buffer (the control), Neo's records of each, and what the report
    may say about Neo's own secrets."""

    layout = P.Layout.UPLOAD

    def setUp(self) -> None:
        super().setUp()
        self.disk = self.inst.plant(origin=P.Origin.FILE)
        self.download = self.inst.plant(origin=P.Origin.DOWNLOAD)
        self.buffer = self.inst.plant(origin=P.Origin.AGENT)

    def echo(self, plant: P.Plant) -> P.Region:
        return echo_of(self.inst, plant)

    def read_back(self, plants: list[P.Plant], run: str = "read1") -> None:
        """The agent reads the page's echoes of `plants` back raw."""
        echoes = [self.echo(p) for p in plants]
        self.proof(f"{run}p", echoes)
        value = {
            r.id: {"ok": f"ECHO {r.anchor} {{{{{r.expected()}}}}}"} for r in echoes
        }
        self.capture(
            run,
            P.ChannelId.TEXT_CONTENT,
            self.reply(value, run),
            run_id=run,
            proofs=(f"{run}p", f"{run}p"),
            regions=tuple(r.id for r in echoes),
            phase=P.Phase.ANY,
        )

    def upload(self, ident: str, channel: P.ChannelId, plant: P.Plant, **reply) -> None:
        (field,) = self.inst.field_regions(plant)
        self.proof(f"{ident}p", [])
        self.capture(
            ident,
            channel,
            self.reply(reply.get("value", 1), ident, reply.get("error")),
            run_id=ident,
            proofs=(f"{ident}p", f"{ident}p"),
            regions=(field.id,),
            phase=P.Phase.ANY,
        )

    def record_of(self, ident: str, run: str, text: str) -> None:
        body = json.dumps({"id": run, "result": text}).encode()
        self.capture(ident, P.ChannelId.RUN, body, run_id=run, phase=P.Phase.ANY)

    def cells(self, channel: P.ChannelId) -> dict[str, tuple]:
        return {
            c.region.id: (c.verdict, c.residual)
            for c in self.judged()
            if c.channel is channel
        }

    def test_a_file_from_neos_disk_read_back_raw_is_by_design(self) -> None:
        """No rule can tell a file's content is sensitive, and the owner serves any
        path: the agent's read of the page's echo is BD-UPLOAD, a downloaded file's as
        much as any other, and the record of that run inherits it. The agent's own
        buffer read back is the control."""
        self.upload("up1", P.ChannelId.UPLOAD, self.disk)
        self.upload("up2", P.ChannelId.UPLOAD, self.download)
        for plant in (self.disk, self.download):
            self.step(1, "uploaded", plant=plant.id, shown=True)
        self.read_back([self.disk, self.download, self.buffer])
        echo = self.echo(self.disk)
        self.record_of("h1", "read1", f"ECHO {echo.anchor} {{{{{echo.expected()}}}}}")
        read = self.cells(P.ChannelId.TEXT_CONTENT)
        self.assertEqual(read[echo.id], (J.Verdict.LEAKED, "BD-UPLOAD"))
        self.assertEqual(
            read[self.echo(self.download).id], (J.Verdict.LEAKED, "BD-UPLOAD")
        )
        self.assertEqual(read[self.echo(self.buffer).id], (J.Verdict.LEAKED, "CONTROL"))
        self.assertEqual(
            self.cells(P.ChannelId.RUN)[echo.id], (J.Verdict.LEAKED, "BD-UPLOAD")
        )
        for plant in (self.disk, self.download):
            (field,) = self.inst.field_regions(plant)
            self.assertEqual(
                self.cells(P.ChannelId.UPLOAD)[field.id],
                (J.Verdict.NOT_APPLICABLE, None),
            )

    def test_an_ordinary_upload_the_page_never_received_is_unproven(self) -> None:
        """Sent is not enough: the page must show the file it was given (the harness's
        "uploaded" step). One that answers sent and never arrives, dropped without a
        word, is as much a functional regression as a refusal: unproven (+8). The
        downloads file here has no step at all, so nothing ever saw it arrive."""
        self.upload("up1", P.ChannelId.UPLOAD, self.disk)
        self.upload("up2", P.ChannelId.UPLOAD, self.download)
        self.step(1, "uploaded", plant=self.disk.id, shown=False)
        judgement = J.judge(self.evidence())
        cells = {
            c.region.id: (c.verdict, c.why)
            for c in judgement.cells
            if c.channel is P.ChannelId.UPLOAD
        }
        for plant in (self.disk, self.download):
            (field,) = self.inst.field_regions(plant)
            self.assertEqual(
                cells[field.id],
                (J.Verdict.INCONCLUSIVE, "ordinary upload not received"),
            )
        self.assertEqual(judgement.exit_code, J.EXIT_INCONCLUSIVE)

    def test_an_ordinary_upload_refused_is_unproven_and_no_leak(self) -> None:
        """Both ordinary files must upload: the deny list covers Neo's own secrets only,
        and agents re-upload what they downloaded (decision 33b). Refusing either is a
        functional regression, not a leak: unproven (+8), never held, never a leak."""
        owner, code = P.DENY_CODE.split("/")
        refused = {"err": "refused", "owner": owner, "code": code}
        for n, plant in enumerate((self.disk, self.download)):
            (field,) = self.inst.field_regions(plant)
            self.upload(f"up{n}", P.ChannelId.UPLOAD, plant, value={field.id: refused})
        judgement = J.judge(self.evidence())
        cells = [c for c in judgement.cells if c.channel is P.ChannelId.UPLOAD]
        self.assertEqual(
            {c.region.plant.origin for c in cells}, {P.Origin.FILE, P.Origin.DOWNLOAD}
        )
        for cell in cells:
            self.assertIs(cell.verdict, J.Verdict.INCONCLUSIVE)
            self.assertTrue(
                cell.why.startswith(f"ordinary upload refused: {P.DENY_CODE}"), cell.why
            )
        self.assertEqual(judgement.exit_code, J.EXIT_INCONCLUSIVE)

    def test_neo_keeping_the_uploaded_file_in_its_record_is_a_new_leak(self) -> None:
        """The upload op answers nothing; if Neo's record of that op holds the file's
        content, Neo kept a value it read from disk, which no agent read explains."""
        self.upload("up1", P.ChannelId.UPLOAD, self.disk)
        self.read_back([self.disk])
        self.record_of("h1", "up1", f"uploaded {self.disk.secret.reveal()}")
        self.assertEqual(
            set(self.cells(P.ChannelId.RUN).values()), {(J.Verdict.LEAKED, None)}
        )

    def test_a_held_upload_is_held_until_served(self) -> None:
        """Today a path upload answers script/method_unavailable (held, UPLOAD-HELD),
        and the prelude refuses a payload (neo/unsupported, not built)."""
        held = {"owner": "script", "code": "method_unavailable", "message": "held"}
        refused = {"owner": "neo", "code": "unsupported", "message": "pass paths"}
        self.upload("up1", P.ChannelId.UPLOAD, self.disk, value=None, error=held)
        self.upload(
            "up2", P.ChannelId.UPLOAD_BUFFER, self.buffer, value=None, error=refused
        )
        (disk,) = self.inst.field_regions(self.disk)
        (buffer,) = self.inst.field_regions(self.buffer)
        self.assertEqual(
            self.cells(P.ChannelId.UPLOAD)[disk.id],
            (J.Verdict.WITHHELD, "UPLOAD-HELD"),
        )
        self.assertEqual(
            self.cells(P.ChannelId.UPLOAD_BUFFER)[buffer.id],
            (J.Verdict.NOT_APPLICABLE, None),
        )

    def test_a_void_probe_keys_its_own_refusal_so_the_code_survives(self) -> None:
        """A refusal the prelude throws (a payload, today) would reach the run as the
        script's own error, so the probe catches it and keys it by the field: not built.
        A run that went through answers {sent: true}: ran."""
        (field,) = self.inst.field_regions(self.buffer)
        code = P.buffer_code(self.inst, field)
        self.assertIn("return {sent: true};", code)
        self.assertIn(f'return {{"{field.id}": refused}};', code)
        refused = {"err": "pass paths", "owner": "neo", "code": "unsupported"}
        self.upload(
            "up1", P.ChannelId.UPLOAD_BUFFER, self.buffer, value={field.id: refused}
        )
        self.upload("up2", P.ChannelId.UPLOAD_BUFFER, self.buffer, value={"sent": True})
        got = {
            c.source: (c.verdict, c.why)
            for c in self.judged()
            if c.channel is P.ChannelId.UPLOAD_BUFFER and c.region.id == field.id
        }
        self.assertEqual(
            got,
            {
                "up1": (J.Verdict.NOT_APPLICABLE, "not built (neo/unsupported)"),
                "up2": (
                    J.Verdict.NOT_APPLICABLE,
                    "ran; the op answers nothing by contract",
                ),
            },
        )
        layouts = {
            P.ChannelId.UPLOAD: P.Layout.UPLOAD,
            P.ChannelId.SET_CONTENT: P.Layout.CONTENT,
        }
        for channel, layout in layouts.items():
            instance = P.make_instance(layout, seed="void", index=3)
            for run in P.plan_runs(P.SPECS[channel], instance):
                with self.subTest(channel=channel):
                    self.assertIn("return {sent: true};", run.code)

    def test_the_report_never_prints_neos_own_secrets(self) -> None:
        """A leak's context is a window of its answer, so a Neo secret beside a leaking
        value could ride into the report: the host token and the fake CAPTCHA key the
        Session mints are redacted wherever they sit."""
        with tempfile.TemporaryDirectory() as root:
            session = S.Session("img", Path(root), seed="s", keep=False)
        secrets = (session.token, session.captcha_key)
        self.upload("up1", P.ChannelId.UPLOAD, self.disk)
        kept = f"uploaded {self.disk.secret.reveal()} {secrets[0]} {secrets[1]}"
        self.record_of("h1", "up1", kept)
        evidence = self.evidence()
        judgement = J.judge(evidence)
        report = J.render_markdown(judgement, evidence)
        self.assertIn("⟨host token⟩ ⟨CAPTCHA key⟩", report, "the context reached them")
        for output in (report, J.render_json(judgement, evidence)):
            for secret in secrets:
                self.assertNotIn(secret, output)
                self.assertNotIn(secret[-16:], output)


class NeoSecretTests(EvidenceBase):
    """Neo's own secrets that the harness mints, the host token and the fake CAPTCHA
    key, are hunted in every answer the judge reads. A hit is a new leak with its exact
    request, whatever the channel; the report and verdicts never print either."""

    def setUp(self) -> None:
        super().setUp()
        with tempfile.TemporaryDirectory() as root:
            session = S.Session("img", Path(root), seed="s", keep=False)
        self.token, self.key = session.token, session.captcha_key

    def disk(self, rel: str, data: bytes) -> None:
        path = self.root / "disk/live" / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    def test_a_secret_in_any_answer_is_a_new_leak(self) -> None:
        """An MCP reply (a cold read of an exit the design masks with L2 only, where a
        plant would be a known residual), a host record, the events stream, a display's
        bytes, the trace store's WAL, claw-sdk's logs and docker's: each hit is a new
        leak in its own row, with nothing to explain it away."""
        token, key = self.token, self.key
        facts = P.ChannelId.FACTS
        self.capture("c1", facts, self.reply({"title": token}, "c1"), run_id="c1")
        self.capture("c2", facts, self.reply({"title": token}, "c2"), run_id="c2")
        record = json.dumps({"id": "c1", "result": f"key {key}"}).encode()
        self.capture("h1", P.ChannelId.RUN, record, run_id="c1", phase=P.Phase.ANY)
        self.stream("s1", P.ChannelId.EVENTS, f"data: {token}\n\n".encode())
        self.stream(
            "s2",
            P.ChannelId.DISPLAY_VIEW,
            b"RFB 003.008\n\x03" + key.encode(),
            role="control",
            instance=self.inst.name,
            target="/v1/display?mode=control",
        )
        self.disk("claw-sdk/trace.db-wal", b"\x37\x7f\x06\x82" + token.encode())
        self.disk("logs/claw-sdk.log.2026-11-05", f"solver {key}\n".encode())
        (self.root / "docker.log").write_text(f"[neo] {token}\n")
        evidence = self.evidence()
        judgement = J.judge(evidence)
        found = {
            (c.channel, c.region.plant.id): (c.verdict, c.residual)
            for c in judgement.cells
            if c.region.plant.origin is P.Origin.NEO
        }
        self.assertEqual(
            set(found),
            {
                (facts, "neo.host-token"),
                (P.ChannelId.RUN, "neo.captcha-key"),
                (P.ChannelId.EVENTS, "neo.host-token"),
                (P.ChannelId.CLIPBOARD_CONTROL, "neo.captcha-key"),
                (P.ChannelId.TRACE_STORE, "neo.host-token"),
                (P.ChannelId.CLAW_LOGS, "neo.captcha-key"),
                (P.ChannelId.DOCKER_LOGS, "neo.host-token"),
            },
        )
        self.assertEqual(set(found.values()), {(J.Verdict.LEAKED, None)})
        self.assertEqual(judgement.exit_code & J.EXIT_NEW_LEAK, J.EXIT_NEW_LEAK)
        report = J.render_markdown(judgement, evidence)
        self.assertIn(f"**{facts}** · file ⟨neo.host-token", report)
        self.assertIn("request: `mcp tools/call run /mcp`", report)
        self.assertIn("evidence: `docker.log`", report)
        self.assertIn("also in 1 more answer: c2", report)
        self.assertIn("⟨host token⟩", report, "the context reached it")
        for output in (report, J.render_json(judgement, evidence)):
            for secret in (token, key):
                self.assertNotIn(secret, output)

    def test_the_harness_requests_are_never_hunted(self) -> None:
        """The token rides in the harness's own requests by design (the bearer header,
        and any body a test gives it): only Neo's answers are searched."""
        body = json.dumps({"authorization": f"Bearer {self.token}"})
        self.capture("c1", P.ChannelId.HARNESS, self.reply(1, "c1"), request=body)
        neo = [c for c in self.judged() if c.region.plant.origin is P.Origin.NEO]
        self.assertEqual(neo, [])


DENY_HEADLINE = "setInputFiles of Neo's own files (decision 33b)"


class DenyTests(EvidenceBase):
    """setInputFiles of Neo's own secret files through judge() (decision 33b): refused
    with DENY_CODE is held; the page reading the file, or a refusal naming its path, is a
    new leak; any other answer is unproven with its reason."""

    layout = P.Layout.UPLOAD
    NOTHING = {
        "files": 0,
        "read": 0,
        "errors": [],
        "matched": False,
        "empty": False,
        "cancelled": False,
        "decoy": False,
    }

    def upload_of(
        self, target: P.NeoFile, via: P.Via, *, front: bool = False
    ) -> P.DenyUpload:
        return next(
            u
            for u in P.deny_uploads(self.inst)
            if (u.probe.target, u.probe.via, u.probe.front) == (target, via, front)
        )

    @staticmethod
    def refusal(key: str, message: str = "refused") -> dict:
        """A refusal as void_run catches it and keys it by the input."""
        owner, code = key.split("/")
        return {"err": message, "owner": owner, "code": code}

    def facts(self, upload: P.DenyUpload, **detail: object) -> dict:
        return {
            "region": upload.region.id,
            "probe": str(upload.probe),
            "named": upload.named,
            "paths": list(upload.paths),
            "refused": upload.probe.refused,
            "front": upload.probe.front,
            "present": True,
            "made": True,
            **detail,
        }

    def probe(
        self,
        upload: P.DenyUpload,
        ident: str,
        *,
        refused: dict | None = None,
        read: dict | None = None,
    ) -> None:
        """One probe as the Session records it: the agent's run (sent, or its refusal
        keyed by the input) and the step beside it with what the oracle found the page
        read, compared inside the container."""
        value = {upload.region.id: refused} if refused else {"sent": True}
        self.capture(
            ident,
            P.ChannelId.UPLOAD_NEO,
            self.reply(value, ident),
            run_id=ident,
            regions=(upload.region.id,),
            phase=P.Phase.ANY,
        )
        checked = {**self.NOTHING, **(read or {})}
        self.step(1, "neo_upload", **self.facts(upload, capture=ident, read=checked))

    def verdicts(self) -> dict[str, tuple]:
        return {
            c.region.id: (c.verdict, c.residual, c.why)
            for c in self.judged()
            if c.channel is P.ChannelId.UPLOAD_NEO
        }

    def test_refused_with_the_deny_code_is_held_and_sets_no_bit(self) -> None:
        """The contract: Neo refuses before reading, with DENY_CODE, naming no path, and
        the page holds nothing. Held by contract: no residual, no exit bit."""
        for n, upload in enumerate(P.deny_uploads(self.inst)):
            self.probe(upload, f"d{n}", refused=self.refusal(P.DENY_CODE))
        judgement = J.judge(self.evidence())
        cells = [c for c in judgement.cells if c.channel is P.ChannelId.UPLOAD_NEO]
        self.assertEqual(len(cells), len(P.DENY_PROBES))
        self.assertEqual(
            {(c.verdict, c.residual, c.why) for c in cells},
            {(J.Verdict.WITHHELD, None, P.DENY_CODE)},
        )
        self.assertEqual(judgement.exit_code, 0)

    # What m5-secrets' live test measured for a directory in a background agent tab: the
    # run times out waiting for `input`, and the input fired `cancel` with no file.
    TIMED_OUT = {
        "err": f"page.setInputFiles: Timeout {P.DIRECTORY_TIMEOUT_MS}ms exceeded.",
        "owner": None,
        "code": None,
    }

    def test_a_symlink_directory_cancelled_in_the_background_is_held(self) -> None:
        """A directory holding a symlink to the token is sent, not refused (the crate's
        walk skips symlinks as Chromium does), and a background tab cancels it: the run
        times out and the page holds no file. That is the contract on an agent tab, so
        it is held, with no residual and no exit bit."""
        upload = self.upload_of(P.NeoFile.HOST_TOKEN, P.Via.LINK_DIRECTORY)
        self.assertFalse(upload.probe.refused)
        self.probe(upload, "d1", refused=self.TIMED_OUT, read={"cancelled": True})
        judgement = J.judge(self.evidence())
        self.assertEqual(
            self.verdicts()[upload.region.id],
            (
                J.Verdict.WITHHELD,
                None,
                "cancelled: Chromium reads no directory in a background tab",
            ),
        )
        self.assertEqual(judgement.exit_code, 0)

    def test_a_cancel_proves_nothing_where_the_deny_list_must_act_or_from_the_front(
        self,
    ) -> None:
        """trace.db's directory must be refused before any read: a cancel there means
        the deny list let it through and only the background tab kept it from the page.
        From the front tab a cancel lands nothing, so nothing shows the symlink was
        skipped. Both are unproven (+8)."""
        trace = self.upload_of(P.NeoFile.TRACE_DB, P.Via.DIRECTORY)
        front = self.upload_of(P.NeoFile.HOST_TOKEN, P.Via.LINK_DIRECTORY, front=True)
        self.probe(trace, "d1", refused=self.TIMED_OUT, read={"cancelled": True})
        self.probe(front, "d2", refused=self.TIMED_OUT, read={"cancelled": True})
        judgement = J.judge(self.evidence())
        verdicts = self.verdicts()
        self.assertEqual(
            verdicts[trace.region.id],
            (
                J.Verdict.INCONCLUSIVE,
                None,
                f"{trace.probe}: cancelled, not refused: only the background tab "
                "kept the page from reading it",
            ),
        )
        self.assertEqual(
            verdicts[front.region.id],
            (
                J.Verdict.INCONCLUSIVE,
                None,
                f"{front.probe}: cancelled in the front tab: nothing landed to show "
                "the symlink skipped",
            ),
        )
        self.assertEqual(judgement.exit_code, J.EXIT_INCONCLUSIVE)

    def test_a_symlink_directory_from_the_front_lands_without_its_target(self) -> None:
        """From the front tab the directory lands: the page reading the decoy beside the
        symlink, and not the token, shows Chromium skipped the symlink. Masked, with no
        exit bit."""
        front = self.upload_of(P.NeoFile.HOST_TOKEN, P.Via.LINK_DIRECTORY, front=True)
        self.probe(front, "d1", read={"files": 1, "read": 1, "decoy": True})
        judgement = J.judge(self.evidence())
        self.assertEqual(
            self.verdicts()[front.region.id],
            (
                J.Verdict.MASKED,
                None,
                f"{front.probe}: sent; the page read the directory's own file, not "
                "the symlinked one",
            ),
        )
        self.assertEqual(judgement.exit_code, 0)

    def test_the_token_read_through_a_symlink_directory_leaks(self) -> None:
        """The page reading the token's bytes leaks, decoy or not; a refusal of a probe
        that should be sent (bringToFront refused, say) is unproven, with its code."""
        front = self.upload_of(P.NeoFile.HOST_TOKEN, P.Via.LINK_DIRECTORY, front=True)
        back = self.upload_of(P.NeoFile.HOST_TOKEN, P.Via.LINK_DIRECTORY)
        both = {"files": 2, "read": 2, "decoy": True, "matched": True}
        self.probe(front, "d1", read=both)
        self.probe(back, "d2", refused=self.refusal("neo/unsupported", "no front"))
        verdicts = self.verdicts()
        self.assertEqual(
            verdicts[front.region.id],
            (J.Verdict.LEAKED, None, f"{front.probe}: the page read its bytes"),
        )
        self.assertEqual(
            verdicts[back.region.id],
            (
                J.Verdict.INCONCLUSIVE,
                None,
                f"{back.probe}: refused with neo/unsupported, not sent: no front",
            ),
        )

    def test_held_while_set_input_files_is_held(self) -> None:
        """Today's image holds setInputFiles whole (script/method_unavailable): held,
        UPLOAD-HELD, until the owner's call lands."""
        upload = self.upload_of(P.NeoFile.HOST_TOKEN, P.Via.SYMLINK)
        self.probe(upload, "d1", refused=self.refusal("script/method_unavailable"))
        judgement = J.judge(self.evidence())
        self.assertEqual(
            self.verdicts()[upload.region.id],
            (J.Verdict.WITHHELD, "UPLOAD-HELD", "script/method_unavailable"),
        )
        self.assertEqual(judgement.exit_code, J.EXIT_KNOWN)

    def test_the_page_reading_the_file_is_a_new_leak_whatever_neo_answered(
        self,
    ) -> None:
        """Neo's answer is not the proof: the oracle found that a file the page read
        hashes the same as the target. Sent, or refused with the right code, that is a
        new leak."""
        hard = self.upload_of(P.NeoFile.TRACE_DB, P.Via.HARD_LINK)
        wal = self.upload_of(P.NeoFile.TRACE_WAL, P.Via.PATH)
        bytes_read = {"files": 1, "read": 1, "matched": True}
        self.probe(hard, "d1", read=bytes_read)
        self.probe(wal, "d2", refused=self.refusal(P.DENY_CODE), read=bytes_read)
        judgement = J.judge(self.evidence())
        verdicts = self.verdicts()
        for upload in (hard, wal):
            self.assertEqual(
                verdicts[upload.region.id],
                (J.Verdict.LEAKED, None, f"{upload.probe}: the page read its bytes"),
            )
        self.assertEqual(judgement.exit_code & J.EXIT_NEW_LEAK, J.EXIT_NEW_LEAK)

    def test_a_refusal_that_names_the_path_leaks_the_path(self) -> None:
        """The refusal must name no path: not where a symlink leads (the token's path),
        nor the path the agent gave. Either is a leak of the path, right code or not."""
        token = P.NEO_FILES[P.NeoFile.HOST_TOKEN]
        link = self.upload_of(P.NeoFile.HOST_TOKEN, P.Via.SYMLINK)
        trace = self.upload_of(P.NeoFile.TRACE_DB, P.Via.DIRECTORY)
        self.probe(
            link, "d1", refused=self.refusal(P.DENY_CODE, f"{token} is Neo's secret")
        )
        self.probe(
            trace, "d2", refused=self.refusal(P.DENY_CODE, f"refused {trace.named}")
        )
        judgement = J.judge(self.evidence())
        verdicts = self.verdicts()
        for upload in (link, trace):
            self.assertEqual(
                verdicts[upload.region.id],
                (
                    J.Verdict.LEAKED,
                    None,
                    f"{upload.probe}: the refusal names its path ({P.DENY_CODE})",
                ),
            )
        self.assertEqual(judgement.exit_code & J.EXIT_NEW_LEAK, J.EXIT_NEW_LEAK)

    def test_any_other_answer_is_unproven_with_its_reason(self) -> None:
        """A wrong code, or an upload that went through with nothing to show for it, is
        no proof either way: unproven (+8), each with its reason."""
        cases = [
            (
                P.NeoFile.HOST_TOKEN,
                P.Via.PATH,
                {"refused": self.refusal("neo/unsupported", "no such op")},
                f"refused with neo/unsupported, not {P.DENY_CODE}: no such op",
            ),
            (
                P.NeoFile.CAPTCHA_KEY,
                P.Via.PATH,
                {"read": {}},
                "sent, but the page's input holds no file",
            ),
            (
                P.NeoFile.COOKIES,
                P.Via.PATH,
                {"read": {"files": 1, "errors": ["NotReadableError"]}},
                "sent, but the page could not read it (NotReadableError)",
            ),
            (
                P.NeoFile.TRACE_WAL,
                P.Via.PATH,
                {"read": {"files": 1, "read": 1, "empty": True}},
                "sent, but it is empty: nothing to read back",
            ),
            (
                P.NeoFile.TRACE_DB,
                P.Via.DIRECTORY,
                {"read": {"files": 3, "read": 3}},
                "sent; the page read 3 of 3 files, none of them its bytes "
                "(it may have changed since)",
            ),
        ]
        for n, (target, via, kw, _) in enumerate(cases):
            self.probe(self.upload_of(target, via), f"d{n}", **kw)
        judgement = J.judge(self.evidence())
        verdicts = self.verdicts()
        for target, via, _, why in cases:
            upload = self.upload_of(target, via)
            with self.subTest(probe=str(upload.probe)):
                self.assertEqual(
                    verdicts[upload.region.id],
                    (J.Verdict.INCONCLUSIVE, None, f"{upload.probe}: {why}"),
                )
        self.assertEqual(judgement.exit_code, J.EXIT_INCONCLUSIVE)

    def test_a_sent_upload_nobody_checked_is_unproven(self) -> None:
        """A sent upload proves nothing until the oracle has looked at the page: a run
        with no step beside it (the harness failed between them) stays unproven."""
        upload = self.upload_of(P.NeoFile.LOGIN_DATA, P.Via.PATH)
        rid = upload.region.id
        self.capture(
            "d1",
            P.ChannelId.UPLOAD_NEO,
            self.reply({"sent": True}, "d1"),
            run_id="d1",
            regions=(rid,),
            phase=P.Phase.ANY,
        )
        self.assertEqual(
            self.verdicts()[rid],
            (
                J.Verdict.INCONCLUSIVE,
                None,
                f"{rid}: sent, but nothing checked what the page read",
            ),
        )

    def test_a_probe_the_harness_could_not_set_up_never_ran(self) -> None:
        """A target missing from the container leaves its deny entry unproven; a hard
        link the filesystem will not make leaves nothing to probe: not applicable."""
        key = self.upload_of(P.NeoFile.CAPTCHA_KEY, P.Via.PATH)
        hard = self.upload_of(P.NeoFile.TRACE_DB, P.Via.HARD_LINK)
        self.step(1, "neo_upload", **self.facts(key, present=False, made=False))
        why = "Invalid cross-device link"
        self.step(2, "neo_upload", **self.facts(hard, made=False, why=why))
        judgement = J.judge(self.evidence())
        verdicts = self.verdicts()
        self.assertEqual(
            verdicts[key.region.id],
            (
                J.Verdict.INCONCLUSIVE,
                None,
                f"{key.probe}: its target is not in this container",
            ),
        )
        self.assertEqual(
            verdicts[hard.region.id],
            (
                J.Verdict.NOT_APPLICABLE,
                None,
                f"{hard.probe}: the harness could not make it ({why})",
            ),
        )
        self.assertEqual(judgement.exit_code, J.EXIT_INCONCLUSIVE)

    def test_the_headline_sums_up_the_deny_list(self) -> None:
        """One line for the owner: how many probes, held by which refusal, and every
        probe that leaked or stayed unproven, by name; and the read-back's control."""
        uploads = P.deny_uploads(self.inst)
        self.probe(uploads[0], "d0", refused=self.refusal(P.DENY_CODE))
        self.probe(uploads[1], "d1", refused=self.refusal(P.DENY_CODE))
        self.probe(uploads[2], "d2", refused=self.refusal("script/method_unavailable"))
        self.probe(uploads[3], "d3", read={"files": 1, "read": 1, "matched": True})
        self.probe(uploads[4], "d4", read={})
        self.step(9, "neo_upload_control", files=1, read=1, matched=True)
        headline = J.judge(self.evidence()).headline
        old = "Neo's own files through setInputFiles (information)"
        self.assertNotIn(old, headline, "the information-only line is gone")
        self.assertEqual(
            headline[DENY_HEADLINE],
            f"5 probes: 2 held ({P.DENY_CODE}), 1 held (script/method_unavailable), "
            f"1 LEAK ({uploads[3].probe}: the page read its bytes), "
            f"1 unproven ({uploads[4].probe}: sent, but the page's input holds no "
            "file); read-back control ok",
        )


class DenySessionTests(unittest.TestCase):
    """leak_sdk.Session.deny_list over a fake oracle and agent: what it records for each
    probe is what decide_neo_upload reads, so the run and the judge cannot drift."""

    NOTHING = {
        "files": 0,
        "read": 0,
        "errors": [],
        "matched": False,
        "empty": False,
        "cancelled": False,
        "decoy": False,
    }

    def deny_list(self, *, control: bool = True) -> tuple[J.Judgement, dict]:
        """Every probe answered sent; the CAPTCHA key missing from the container, the
        hard link refused by the filesystem, and only the token's own path read back."""
        root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, root)
        session = S.Session("img", root, seed="s", keep=False)
        instance = P.make_instance(P.Layout.UPLOAD, seed="s", index=1)
        session.instances[instance.name] = instance
        session._write_manifest()
        uploads = {
            (u.probe.target, u.probe.via, u.probe.front): u
            for u in P.deny_uploads(instance)
        }
        token = uploads[(P.NeoFile.HOST_TOKEN, P.Via.PATH, False)]
        missing = P.NEO_FILES[P.NeoFile.CAPTCHA_KEY]
        nothing = self.NOTHING

        class Oracle:
            def call(self, op: str, **params: object) -> dict:
                ok = {"id": 1, "ok": True}
                if op == "digest_control":
                    return {**ok, **nothing, "files": 1, "read": 1, "matched": control}
                if op == "prepare_upload":
                    if params["target"] == missing:
                        return {**ok, "present": False}
                    if any(link[0] == "hard link" for link in params["links"]):
                        why = "Invalid cross-device link"
                        return {**ok, "present": True, "made": False, "why": why}
                    return {**ok, "present": True, "made": True}
                assert op == "read_back", op
                read = params["region"] == token.region.id
                # The page reads the decoy whenever the session named one.
                decoy = params["decoy"] is not None
                facts = {"files": 1, "read": 1, "matched": read, "decoy": decoy}
                return {**ok, **nothing, **facts}

        class Agent:
            def run(self, code: str, meta: dict, timeout_ms: int) -> tuple[str, bytes]:
                return code, EvidenceBase.reply({"sent": True}, meta["requestId"])

        session.oracle, session.agent = Oracle(), Agent()
        session.deny_list(S.Page(instance, 1))
        return J.judge(J.Evidence.load(root)), uploads

    def test_each_probe_records_what_the_judge_reads(self) -> None:
        """A target missing here and a link the filesystem refused are judged from
        their steps, with no run; a run that went through is judged by what the oracle
        found the page read: the token's bytes leak, a symlink directory's decoy alone
        is masked (the session passes the decoy and the probe's expectations through),
        anything else stays unproven."""
        judgement, uploads = self.deny_list()
        verdicts = {
            c.region.id: (c.verdict, c.why)
            for c in judgement.cells
            if c.channel is P.ChannelId.UPLOAD_NEO
        }
        self.assertEqual(len(verdicts), len(P.DENY_PROBES))
        token = uploads[(P.NeoFile.HOST_TOKEN, P.Via.PATH, False)]
        key = uploads[(P.NeoFile.CAPTCHA_KEY, P.Via.PATH, False)]
        hard = uploads[(P.NeoFile.TRACE_DB, P.Via.HARD_LINK, False)]
        links = [u for u in uploads.values() if u.probe.via is P.Via.LINK_DIRECTORY]
        self.assertEqual(len(links), 2)
        special = {
            token.region.id: (
                J.Verdict.LEAKED,
                f"{token.probe}: the page read its bytes",
            ),
            key.region.id: (
                J.Verdict.INCONCLUSIVE,
                f"{key.probe}: its target is not in this container",
            ),
            hard.region.id: (
                J.Verdict.NOT_APPLICABLE,
                f"{hard.probe}: the harness could not make it "
                "(Invalid cross-device link)",
            ),
            **{
                u.region.id: (
                    J.Verdict.MASKED,
                    f"{u.probe}: sent; the page read the directory's own file, not "
                    "the symlinked one",
                )
                for u in links
            },
        }
        for upload in uploads.values():
            want = special.get(upload.region.id) or (
                J.Verdict.INCONCLUSIVE,
                f"{upload.probe}: sent; the page read 1 of 1 files, none of them its "
                "bytes (it may have changed since)",
            )
            with self.subTest(probe=str(upload.probe)):
                self.assertEqual(verdicts[upload.region.id], want)
        headline = judgement.headline[DENY_HEADLINE]
        self.assertTrue(headline.endswith("; read-back control ok"), headline)
        self.assertEqual(judgement.failures, ())

    def test_a_failed_read_back_control_is_a_harness_failure(self) -> None:
        """If the page's digest misses a file the oracle set itself, no "sent" answer
        can be judged by what the page read: the table is not to be trusted (+16)."""
        judgement, _ = self.deny_list(control=False)
        self.assertEqual(judgement.exit_code & J.EXIT_HARNESS, J.EXIT_HARNESS)
        headline = judgement.headline[DENY_HEADLINE]
        self.assertTrue(headline.endswith("; read-back control FAILED"), headline)


class FinishTests(unittest.TestCase):
    """leak_sdk.Session.finish with a container whose every collection breaks."""

    def test_a_failed_collection_is_a_harness_failure_and_cleanup_goes_on(self) -> None:
        """finish() gathers host records, the profile scan, a live and a stopped copy
        of the state dir, and the logs. One that raises leaves that channel out of the
        table, so it must be a failed Step (+16); the next collection and the
        container's removal still run."""
        removed: list[str] = []

        def broken(*_args: object, **_kw: object) -> bytes:
            raise RuntimeError("the container is gone")

        class Docker:
            exec = logs = cp_out = staticmethod(broken)

            def stop(self) -> None:
                pass

            def remove(self) -> None:
                removed.append("container")

        class Events:
            def __init__(self, blob: Path) -> None:
                blob.write_bytes(b"")
                self.blob, self.opened_ms, self.chunks = blob, 0, []
                self.closed, self.status = (1, "harness"), 200

            def stop(self) -> None:
                pass

        class Oracle:
            call = staticmethod(broken)

            def close(self) -> None:
                pass

        with tempfile.TemporaryDirectory() as root:
            out = Path(root)
            session = S.Session("img", out, seed="s", keep=False)
            session.docker, session.oracle = Docker(), Oracle()
            session.host = type("Host", (), {"call": staticmethod(broken)})()
            session.events = Events(out / "streams/events.bin")
            session.token_file.write_text("token")
            session.captcha_key_file.write_text("key")
            session.finish()
            evidence = J.Evidence.load(out)
            failures = "\n".join(evidence.failures)
            for channel in ("records", "profile", "live", "logs", "stopped"):
                self.assertIn(channel, failures)
            self.assertEqual(len(evidence.failures), 5)
            exit_code = J.judge(evidence).exit_code
            self.assertEqual(exit_code & J.EXIT_HARNESS, J.EXIT_HARNESS)
            self.assertEqual(removed, ["container"])
            self.assertFalse(session.token_file.exists())
            self.assertFalse(session.captcha_key_file.exists())


class ResidualTests(unittest.TestCase):
    def setUp(self) -> None:
        self.inst = P.make_instance(P.Layout.ECHOES, seed="res", index=1)

    def leak(self, kind: P.Kind, loc: P.Location, phase: P.Phase, **kw) -> J.Cell:
        plant = self.inst.plant(kind, loc)
        region = echo_of(self.inst, plant)
        hit = J.Hit(plant.id, region.id, J.Form.PLAIN, 0, "")
        return cell(obs(region, phase=phase, hit=hit, **kw), run=kw.get("run_id"))

    def test_fx_f_labels_a_cold_same_site_echo_and_not_a_warm_one(self) -> None:
        cold, warm = (
            self.leak(P.Kind.PASSWORD, P.Location.SAME_SITE, p)
            for p in (P.Phase.COLD, P.Phase.WARM)
        )
        labelled = J.assign_residuals([cold, warm])
        self.assertEqual([c.residual for c in labelled], ["FX-F", None])

    def test_a_cold_cross_site_echo_is_xsite_and_a_short_one_by_design(self) -> None:
        xo = self.leak(P.Kind.PASSWORD, P.Location.CROSS_SITE, P.Phase.COLD)
        cvc = self.leak(P.Kind.CC_CSC, P.Location.CROSS_SITE, P.Phase.COLD)
        self.assertEqual(
            [c.residual for c in J.assign_residuals([xo, cvc])], ["XSITE", "BD-SHORT"]
        )

    def test_a_record_copy_inherits_the_agent_leak_residual(self) -> None:
        agent = self.leak(
            P.Kind.PASSWORD, P.Location.NESTED, P.Phase.COLD, run_id="run1"
        )
        record = dataclasses.replace(
            agent, id="h9:x", channel=P.ChannelId.RUN, source="h9", phase=P.Phase.ANY
        )
        labelled = J.assign_residuals([agent, record])
        self.assertEqual(labelled[1].residual, "FX-F")
        self.assertEqual(labelled[1].derived_from, agent.id)

    def test_a_record_of_a_run_inherits_only_from_that_run(self) -> None:
        """Run 1 leaked the plant cold (XSITE); run 2's agent answer masked it, yet run
        2's host record holds it raw. That copy repeats no agent leak of its own run, so
        it is new, whatever run 1 leaked."""
        earlier = self.leak(
            P.Kind.PASSWORD, P.Location.CROSS_SITE, P.Phase.COLD, run_id="r1"
        )
        masked = dataclasses.replace(
            earlier,
            id="c2:x",
            phase=P.Phase.WARM,
            verdict=J.Verdict.MASKED,
            run_id="r2",
            hit=None,
        )
        record = dataclasses.replace(
            earlier, id="h9:x", channel=P.ChannelId.RUN, source="h9", run_id="r2"
        )
        labelled = J.assign_residuals([earlier, masked, record])
        self.assertEqual(
            [c.residual for c in labelled], ["XSITE", None, None], "run 2 has no leak"
        )
        exit_code = J.Judgement(tuple(labelled), (), {}).exit_code
        self.assertEqual(exit_code & J.EXIT_NEW_LEAK, J.EXIT_NEW_LEAK)

    def test_an_unattributed_disk_hit_is_new_unless_every_agent_leak_agrees(
        self,
    ) -> None:
        a = self.leak(P.Kind.PASSWORD, P.Location.CROSS_SITE, P.Phase.COLD, run_id="r1")
        b = dataclasses.replace(
            self.leak(
                P.Kind.PASSWORD, P.Location.CROSS_SITE, P.Phase.JOURNALED, run_id="r2"
            ),
            id="c2",
        )
        disk = dataclasses.replace(
            a,
            id="disk:x",
            channel=P.ChannelId.TRACE_STORE,
            source="disk",
            run_id=None,
            phase=P.Phase.ANY,
        )
        self.assertEqual(J.assign_residuals([a, disk])[1].residual, "XSITE")
        self.assertIsNone(J.assign_residuals([a, b, disk])[2].residual)
        orphan = self.leak(P.Kind.OTP, P.Location.MAIN, P.Phase.ANY)
        orphan = dataclasses.replace(orphan, channel=P.ChannelId.DOCKER_LOGS)
        self.assertIsNone(J.assign_residuals([orphan])[0].residual)

    def test_held_script_is_a_known_residual(self) -> None:
        (field,) = self.inst.field_regions(
            self.inst.plant(P.Kind.PASSWORD, P.Location.MAIN)
        )
        held = cell(
            obs(
                field,
                channel=P.ChannelId.LOCATOR_EVALUATE,
                refusal=J.Refusal("script", "method_unavailable"),
            )
        )
        (labelled,) = J.assign_residuals([held])
        self.assertEqual(
            (labelled.verdict, labelled.residual), (J.Verdict.WITHHELD, "EVAL-HELD")
        )


class ReportTests(unittest.TestCase):
    def setUp(self) -> None:
        self.inst = P.make_instance(P.Layout.ECHOES, seed="rep", index=1)

    def judgement(
        self, cells: list[J.Cell], failures: tuple[str, ...] = ()
    ) -> J.Judgement:
        return J.Judgement(tuple(J.assign_residuals(cells)), failures, {})

    def test_exit_bits(self) -> None:
        (pw_field,) = self.inst.field_regions(
            self.inst.plant(P.Kind.PASSWORD, P.Location.MAIN)
        )
        (plain_field,) = self.inst.field_regions(
            self.inst.plant(P.Kind.PLAIN, P.Location.MAIN)
        )
        masked = cell(
            obs(pw_field, slot=J.SlotState.MASKED, channel=P.ChannelId.INPUT_VALUE)
        )
        control = cell(
            obs(plain_field, slot=J.SlotState.RAW, channel=P.ChannelId.INPUT_VALUE)
        )
        new = cell(obs(pw_field, slot=J.SlotState.RAW, channel=P.ChannelId.INPUT_VALUE))
        unproven = cell(obs(pw_field, present=False, channel=P.ChannelId.INPUT_VALUE))
        xs = self.inst.plant(P.Kind.PASSWORD, P.Location.CROSS_SITE)
        known = cell(
            obs(echo_of(self.inst, xs), hit=J.Hit(xs.id, None, J.Form.PLAIN, 0, ""))
        )
        self.assertEqual(self.judgement([masked, control]).exit_code, 0)
        self.assertEqual(self.judgement([masked, new]).exit_code, 1)
        self.assertEqual(self.judgement([known]).exit_code, 4)
        self.assertEqual(self.judgement([unproven]).exit_code, 8)
        self.assertEqual(self.judgement([masked], ("echoes: boom",)).exit_code, 16)
        self.assertEqual(self.judgement([new, known, unproven], ("x",)).exit_code, 29)

    def test_worst_verdict_wins_a_table_cell(self) -> None:
        (field,) = self.inst.field_regions(
            self.inst.plant(P.Kind.PASSWORD, P.Location.MAIN)
        )
        masked = cell(
            obs(field, slot=J.SlotState.MASKED, channel=P.ChannelId.INPUT_VALUE)
        )
        raw = cell(obs(field, slot=J.SlotState.RAW, channel=P.ChannelId.INPUT_VALUE))
        grid = J.table(self.judgement([masked, raw, masked]))
        self.assertIs(
            grid[P.ChannelId.INPUT_VALUE][P.Kind.PASSWORD].verdict, J.Verdict.LEAKED
        )

    def test_a_context_never_shows_a_slot_whose_anchor_it_cuts_off(self) -> None:
        """A leak's context is a window of the answer. A short slot near its edge loses
        its anchor to the cut, yet its digits must not show."""
        cvc = echo_of(self.inst, self.inst.plant(P.Kind.CC_CSC, P.Location.MAIN))
        otp = self.inst.plant(P.Kind.OTP, P.Location.MAIN).secret.reveal()
        detector = J.Detector([self.inst])
        for gap in range(30, 70):
            text = f"ECHO {cvc.anchor} {{{{{cvc.expected()}}}}}{'-' * gap}{otp} tail"
            ((context),) = {h.context for h in detector.hits(text)}
            with self.subTest(gap=gap):
                self.assertNotIn(cvc.expected(), context)
                self.assertNotIn(otp, context)

    def test_a_context_cut_before_its_echo_prefix_is_still_redacted(self) -> None:
        cvc = echo_of(self.inst, self.inst.plant(P.Kind.CC_CSC, P.Location.MAIN))
        text = f"HO {cvc.anchor} {{{{{cvc.expected()}}}}} tail"
        self.assertNotIn(
            f"{{{{{cvc.expected()}}}}}", J.Detector([self.inst]).redact(text)
        )


if __name__ == "__main__":
    unittest.main()
