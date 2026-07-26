#!/usr/bin/env python3
"""Tests for Slack build lifecycle notifications."""

import os
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from bos_build.core.context import ArtifactRegistry
from bos_build.core.events import RunFinished, RunStarted, StepFinished, StepStarted
from bos_build.core.products import get_product_descriptor
from bos_build.lib.notify import (
    COLOR_BLUE,
    COLOR_GREEN,
    COLOR_RED,
    Notifier,
    SlackRunSubscriber,
    format_duration,
    slack_subscriber,
)


class _FakeNotifier:
    def __init__(self):
        self.messages = []

    def notify(
        self,
        title,
        body,
        details=None,
        color=COLOR_GREEN,
        wait=False,
        footer=None,
    ):
        self.messages.append(
            {
                "title": title,
                "body": body,
                "details": details,
                "color": color,
                "wait": wait,
                "footer": footer,
            }
        )


class _RaisingNotifier:
    def notify(self, *args, **kwargs):
        raise RuntimeError("notify exploded")


def _ctx(product="browseros", arch="arm64", release_version="", semantic_version="1.2.3"):
    return SimpleNamespace(
        product=get_product_descriptor(product),
        architecture=arch,
        release_version=release_version,
        semantic_version=semantic_version,
        artifact_registry=ArtifactRegistry(),
    )


class FormatDurationTest(unittest.TestCase):
    def test_format_duration_humanizes_seconds(self):
        self.assertEqual(format_duration(45.3), "45s")
        self.assertEqual(format_duration(5006), "83m 26s")
        self.assertEqual(format_duration(4980), "83m")
        self.assertEqual(format_duration(0.4), "0s")
        self.assertEqual(format_duration(59.7), "1m")


class SlackRunSubscriberTest(unittest.TestCase):
    def test_success_sequence_rolls_up_phase_transitions_and_terminal(self):
        fake = _FakeNotifier()
        sub = SlackRunSubscriber(_ctx(), notifier=fake)

        sub(RunStarted(run="build", steps=("clean", "compile", "sign_macos")))
        sub(StepStarted(run="build", step="clean", phase="setup"))
        sub(
            StepFinished(
                run="build",
                step="clean",
                phase="setup",
                status="success",
                duration=12.2,
            )
        )
        sub(StepStarted(run="build", step="compile", phase="build"))
        sub(
            StepFinished(
                run="build",
                step="compile",
                phase="build",
                status="success",
                duration=5006,
            )
        )
        sub(StepStarted(run="build", step="sign_macos", phase="sign"))
        sub(
            StepFinished(
                run="build",
                step="sign_macos",
                phase="sign",
                status="success",
                duration=7.8,
            )
        )
        sub(RunFinished(run="build", status="success", duration=5026.4))

        self.assertEqual(
            [m["title"].split(" — ", 1)[0] for m in fake.messages],
            [
                "🚀 Build started",
                "✅ Setup done (12s) → Build",
                "✅ Build done (83m 26s) → Sign",
                "🏁 Build completed",
            ],
        )
        self.assertEqual(fake.messages[0]["color"], COLOR_BLUE)
        self.assertIn("setup → build → sign", fake.messages[0]["body"])
        self.assertEqual(fake.messages[-1]["body"], "Completed in 83m 46s")
        self.assertEqual(fake.messages[-1]["color"], COLOR_GREEN)
        self.assertTrue(fake.messages[-1]["wait"])
        self.assertIsNone(fake.messages[-1]["details"])

    def test_success_includes_release_links_artifacts_field(self):
        ctx = _ctx()
        ctx.artifact_registry.add(
            "release_links",
            [
                ("BrowserOS.dmg", "https://example.test/BrowserOS.dmg"),
                ("release.json", "https://example.test/release.json"),
            ],
        )
        fake = _FakeNotifier()
        sub = SlackRunSubscriber(ctx, notifier=fake)

        sub(RunStarted(run="build", steps=("compile",)))
        sub(StepStarted(run="build", step="compile", phase="build"))
        sub(
            StepFinished(
                run="build",
                step="compile",
                phase="build",
                status="success",
                duration=1.0,
            )
        )
        sub(RunFinished(run="build", status="success", duration=2.0))

        self.assertEqual(
            fake.messages[-1]["details"],
            {
                "Artifacts": (
                    "<https://example.test/BrowserOS.dmg|BrowserOS.dmg>\n"
                    "<https://example.test/release.json|release.json>"
                )
            },
        )

    def test_malformed_release_links_do_not_drop_terminal_success(self):
        for release_links in (["not-a-pair"], object()):
            with self.subTest(release_links=type(release_links).__name__):
                ctx = _ctx()
                ctx.artifact_registry.add("release_links", release_links)
                fake = _FakeNotifier()
                sub = SlackRunSubscriber(ctx, notifier=fake)

                sub(RunFinished(run="build", status="success", duration=2.0))

                self.assertEqual(len(fake.messages), 1)
                self.assertEqual(
                    fake.messages[0]["title"].split(" — ", 1)[0],
                    "🏁 Build completed",
                )
                self.assertIsNone(fake.messages[0]["details"])

    def test_notifier_failure_propagates_to_runner_guard(self):
        sub = SlackRunSubscriber(_ctx(), notifier=_RaisingNotifier())

        with self.assertRaisesRegex(RuntimeError, "notify exploded"):
            sub(RunStarted(run="build", steps=("compile",)))

    def test_failure_names_step_phase_error_and_waits(self):
        fake = _FakeNotifier()
        sub = SlackRunSubscriber(_ctx(), notifier=fake)

        sub(RunStarted(run="build", steps=("compile", "sign_macos")))
        sub(StepStarted(run="build", step="compile", phase="build"))
        sub(
            StepFinished(
                run="build",
                step="compile",
                phase="build",
                status="failed",
                duration=83.0,
                error="ninja failed",
            )
        )
        sub(RunFinished(run="build", status="failed", duration=84.0, error="compile: ninja failed"))

        self.assertEqual(len(fake.messages), 2)
        terminal = fake.messages[-1]
        self.assertIn("❌ Build FAILED at 'compile' (Build phase)", terminal["title"])
        self.assertIn("BrowserOS v1.2.3", terminal["title"])
        self.assertEqual(terminal["body"], "Terminated after 1m 24s")
        self.assertEqual(terminal["details"], {"Error": "ninja failed"})
        self.assertEqual(terminal["color"], COLOR_RED)
        self.assertTrue(terminal["wait"])

    def test_failure_without_failed_step_falls_back_to_run_error(self):
        fake = _FakeNotifier()
        sub = SlackRunSubscriber(_ctx(), notifier=fake)

        sub(RunStarted(run="build", steps=("missing_step",)))
        sub(RunFinished(run="build", status="failed", duration=5.0, error="unknown step"))

        terminal = fake.messages[-1]
        self.assertEqual(terminal["title"].split(" — ", 1)[0], "❌ Build FAILED")
        self.assertEqual(terminal["details"], {"Error": "unknown step"})

    def test_interrupted_names_current_step_and_waits(self):
        fake = _FakeNotifier()
        sub = SlackRunSubscriber(_ctx(), notifier=fake)

        sub(RunStarted(run="build", steps=("compile",)))
        sub(StepStarted(run="build", step="compile", phase="build"))
        sub(RunFinished(run="build", status="interrupted", duration=65.0, error="Interrupted by user"))

        terminal = fake.messages[-1]
        self.assertIn("🛑 Build interrupted at 'compile'", terminal["title"])
        self.assertEqual(terminal["body"], "after 1m 5s")
        self.assertEqual(terminal["color"], COLOR_RED)
        self.assertTrue(terminal["wait"])

    def test_interrupted_between_steps_omits_completed_step_name(self):
        fake = _FakeNotifier()
        sub = SlackRunSubscriber(_ctx(), notifier=fake)

        sub(StepStarted(run="build", step="compile", phase="build"))
        sub(
            StepFinished(
                run="build",
                step="compile",
                phase="build",
                status="success",
                duration=1.0,
            )
        )
        sub(RunFinished(run="build", status="interrupted", duration=65.0))

        terminal = fake.messages[-1]
        self.assertIn("🛑 Build interrupted —", terminal["title"])
        self.assertNotIn("at 'compile'", terminal["title"])

    def test_unregistered_empty_phase_steps_emit_start_and_terminal_only(self):
        fake = _FakeNotifier()
        sub = SlackRunSubscriber(_ctx(), notifier=fake)

        sub(RunStarted(run="release", steps=("GithubModule",)))
        sub(StepStarted(run="release", step="GithubModule", phase=""))
        sub(
            StepFinished(
                run="release",
                step="GithubModule",
                phase="",
                status="success",
                duration=3.0,
            )
        )
        sub(RunFinished(run="release", status="success", duration=4.0))

        self.assertEqual(len(fake.messages), 2)
        self.assertEqual(fake.messages[0]["title"].split(" — ", 1)[0], "🚀 Release started")
        self.assertEqual(fake.messages[0]["body"], "")
        self.assertEqual(fake.messages[1]["title"].split(" — ", 1)[0], "🏁 Release completed")

    def test_factory_with_unset_webhook_is_silent_noop(self):
        with patch.dict(os.environ, {}, clear=True):
            sub = slack_subscriber(_ctx())
            sub(RunStarted(run="build", steps=("compile",)))
            sub(RunFinished(run="build", status="success", duration=1.0))

            notifier = Notifier()
            self.assertFalse(notifier.enabled)
            notifier.notify("title", "body", wait=True)

    def test_browserclaw_identity_and_footer_use_context_product(self):
        fake = _FakeNotifier()
        sub = SlackRunSubscriber(_ctx(product="browserclaw"), notifier=fake)

        sub(RunStarted(run="build", steps=("compile",)))

        self.assertIn("BrowserClaw", fake.messages[0]["title"])
        self.assertIn("BrowserClaw Build", fake.messages[0]["footer"])


if __name__ == "__main__":
    unittest.main()
