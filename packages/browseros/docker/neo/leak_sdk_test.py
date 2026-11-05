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
import re
import tempfile
import unittest
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


def run_instances(n: int = 12) -> list[P.Instance]:
    return [
        P.make_instance(layout, seed="unit", index=i * 10 + j)
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

    def test_registrable_counts_typed_prefixes_and_skips_short_values(self) -> None:
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
        self.assertEqual(P.registrable(pw, typed=True), 10)

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
        self.assertIn(P.ChannelId.SET_CONTENT, P.SCRIPT_READS)

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


class EvidenceTests(unittest.TestCase):
    """judge() over a synthetic evidence directory: presence around the probe, record
    copies, a value only the disk holds."""

    def setUp(self) -> None:
        self.inst = P.make_instance(P.Layout.ECHOES, seed="ev", index=1)
        self.others: list[P.Instance] = []  # more instances of the same run
        self.root = Path(tempfile.mkdtemp())
        (self.root / "blobs").mkdir()
        self.records: list[str] = []

    def tearDown(self) -> None:
        import shutil

        shutil.rmtree(self.root)

    def echo(self, loc: P.Location) -> P.Region:
        return echo_of(self.inst, self.inst.plant(P.Kind.PASSWORD, loc))

    def proof(self, ident: str, regions: list[P.Region]) -> None:
        digests = {r.id: (J.fnv1a(r.expected()), len(r.expected())) for r in regions}
        proof = J.Proof(ident, self.inst.name, 0, digests, ("main",))
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
            0,
            ((0, 0),),
            (1, "harness"),
            **kw,
        )
        self.records.append(J.encode(stream))

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

    def reply(self, value: object, run: str) -> bytes:
        meta = {"id": run, "status": "ok", "effects": "none", "unknown": []}
        result = {
            "content": [{"type": "text", "text": json.dumps(value)}],
            "structuredContent": {"value": value},
            "isError": False,
            "_meta": {"com.browseros.neo/run": meta},
        }
        return json.dumps({"jsonrpc": "2.0", "id": 1, "result": result}).encode()

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
