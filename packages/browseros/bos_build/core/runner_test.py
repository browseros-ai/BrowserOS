#!/usr/bin/env python3
"""Tests for the unified runner's lifecycle event contract."""

import unittest
from types import SimpleNamespace

from bos_build.core.events import RunFinished, RunStarted, StepFinished, StepStarted
from bos_build.core.runner import RunResult, StepExecutionError, run
from bos_build.core.step import Step, ValidationError


class _Recorder:
    def __init__(self):
        self.events = []

    def __call__(self, event):
        self.events.append(event)

    def finished(self):
        return [e for e in self.events if isinstance(e, RunFinished)]


class _OkStep(Step):
    name = "ok_step"

    def validate(self, context):
        pass

    def execute(self, context):
        context.executed.append(self.name)


class _BoomStep(Step):
    name = "boom_step"

    def validate(self, context):
        pass

    def execute(self, context):
        raise RuntimeError("boom")


class _InvalidStep(Step):
    name = "invalid_step"

    def validate(self, context):
        raise ValidationError("not ready")

    def execute(self, context):
        raise AssertionError("execute must not run after failed validation")


class _InterruptStep(Step):
    name = "interrupt_step"

    def validate(self, context):
        pass

    def execute(self, context):
        raise KeyboardInterrupt()


def _ctx():
    return SimpleNamespace(executed=[], artifact_registry=None)


class RunnerEventTest(unittest.TestCase):
    def test_success_emits_run_finished_exactly_once(self):
        rec = _Recorder()
        result = run(_ctx(), [_OkStep(), _OkStep()], name="r", subscribers=(rec,))

        self.assertIsInstance(result, RunResult)
        self.assertEqual(result.status, "success")
        self.assertEqual(len(rec.finished()), 1)
        self.assertEqual(rec.finished()[0].status, "success")
        self.assertIsInstance(rec.events[0], RunStarted)
        self.assertEqual(
            [e.status for e in rec.events if isinstance(e, StepFinished)],
            ["success", "success"],
        )

    def test_step_failure_emits_failed_run_finished_exactly_once(self):
        rec = _Recorder()
        with self.assertRaises(StepExecutionError):
            run(_ctx(), [_OkStep(), _BoomStep(), _OkStep()], name="r", subscribers=(rec,))

        self.assertEqual(len(rec.finished()), 1)
        self.assertEqual(rec.finished()[0].status, "failed")
        self.assertIn("boom", rec.finished()[0].error)
        # third step never started
        started = [e.step for e in rec.events if isinstance(e, StepStarted)]
        self.assertEqual(started, ["ok_step", "boom_step"])

    def test_validation_failure_stops_before_execute(self):
        rec = _Recorder()
        ctx = _ctx()
        with self.assertRaisesRegex(StepExecutionError, "validation failed"):
            run(ctx, [_InvalidStep()], name="r", subscribers=(rec,))

        self.assertEqual(ctx.executed, [])
        self.assertEqual(rec.finished()[0].status, "failed")

    def test_interrupt_emits_interrupted_run_finished_and_reraises(self):
        rec = _Recorder()
        with self.assertRaises(KeyboardInterrupt):
            run(_ctx(), [_InterruptStep()], name="r", subscribers=(rec,))

        self.assertEqual(len(rec.finished()), 1)
        self.assertEqual(rec.finished()[0].status, "interrupted")

    def test_subscriber_exception_does_not_break_run(self):
        def bad_subscriber(event):
            raise RuntimeError("subscriber exploded")

        result = run(_ctx(), [_OkStep()], name="r", subscribers=(bad_subscriber,))
        self.assertEqual(result.status, "success")

    def test_unknown_step_name_raises(self):
        with self.assertRaisesRegex(StepExecutionError, "unknown step"):
            run(_ctx(), ["definitely_not_a_step"], name="r", available={})

    def test_phase_carried_on_events(self):
        rec = _Recorder()

        class PhaseStep(_OkStep):
            phase = "build"

        run(_ctx(), [PhaseStep()], name="r", subscribers=(rec,))
        started = [e for e in rec.events if isinstance(e, StepStarted)]
        finished = [e for e in rec.events if isinstance(e, StepFinished)]
        self.assertEqual(started[0].phase, "build")
        self.assertEqual(finished[0].phase, "build")


if __name__ == "__main__":
    unittest.main()
