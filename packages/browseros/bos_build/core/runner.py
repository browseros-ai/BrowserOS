#!/usr/bin/env python3
"""Unified pipeline runner.

The single execution path for every CLI (build, release, ota, dev):
resolves step names via the registry, runs validate + execute per step,
tracks timing, and emits lifecycle events. RunFinished is emitted in a
finally block so terminal notifications fire on success, failure, AND
interrupt — the historic bug was end-notifications racing process exit.

Declared `requires` artifacts are checked against the artifact registry
but only warn when absent: partial pipelines (`--modules sign_macos` on
an existing build) legitimately start with an empty in-process registry
and resolve artifacts from disk.
"""

import time
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Sequence, Type, Union

from .events import (
    Event,
    RunFinished,
    RunStarted,
    RunStatus,
    StepFinished,
    StepStarted,
    Subscriber,
)
from .step import Step, ValidationError, all_steps
from ..lib.utils import log_error, log_info, log_success, log_warning


class StepExecutionError(Exception):
    """A step failed validation or execution; carries the step name."""

    def __init__(self, step_name: str, message: str):
        self.step_name = step_name
        super().__init__(f"{step_name}: {message}")


@dataclass
class StepResult:
    name: str
    status: RunStatus
    duration: float


@dataclass
class RunResult:
    name: str
    status: RunStatus
    duration: float
    steps: List[StepResult] = field(default_factory=list)
    error: Optional[str] = None


def run(
    ctx,
    steps: Sequence[Union[str, Step]],
    *,
    name: str = "build",
    subscribers: Sequence[Subscriber] = (),
    available: Optional[Dict[str, Type[Step]]] = None,
) -> RunResult:
    """Run steps in order against ctx, emitting lifecycle events.

    Accepts step names (resolved via `available` or the global registry)
    or pre-built Step instances (for parameterized single-step runs like
    the release CLI's DownloadModule(os_filter=...)).

    Raises StepExecutionError on the first failing step and re-raises
    KeyboardInterrupt; in every case RunFinished has already been emitted.
    """
    resolved = _resolve(steps, available)
    step_names = tuple(_step_name(s) for s in resolved)

    results = RunResult(name=name, status="success", duration=0.0)
    start = time.time()
    _emit(subscribers, RunStarted(run=name, steps=step_names))

    try:
        for step in resolved:
            step_name = _step_name(step)
            log_info(f"\n{'=' * 70}")
            log_info(f"🔧 Running step: {step_name}")
            log_info(f"{'=' * 70}")

            _emit(subscribers, StepStarted(run=name, step=step_name, phase=step.phase))
            step_start = time.time()
            _warn_missing_requires(ctx, step, step_name)

            try:
                step.validate(ctx)
            except ValidationError as e:
                _finish_step(subscribers, results, name, step, step_start, error=str(e))
                log_error(f"Validation failed for {step_name}: {e}")
                raise StepExecutionError(step_name, f"validation failed: {e}") from e

            try:
                step.execute(ctx)
            except Exception as e:
                _finish_step(subscribers, results, name, step, step_start, error=str(e))
                log_error(f"Step {step_name} failed: {e}")
                raise StepExecutionError(step_name, str(e)) from e

            duration = _finish_step(subscribers, results, name, step, step_start)
            log_success(f"Step {step_name} completed in {duration:.1f}s")

        _log_run_success(start)
        return results

    except KeyboardInterrupt:
        results.status = "interrupted"
        results.error = "Interrupted by user"
        log_error("\n❌ Pipeline interrupted")
        raise
    except StepExecutionError as e:
        results.status = "failed"
        results.error = str(e)
        raise
    except Exception as e:
        results.status = "failed"
        results.error = str(e)
        log_error(f"\n❌ Pipeline failed: {e}")
        raise StepExecutionError(name, str(e)) from e
    finally:
        results.duration = time.time() - start
        _emit(
            subscribers,
            RunFinished(
                run=name,
                status=results.status,
                duration=results.duration,
                error=results.error,
            ),
        )


def _resolve(
    steps: Sequence[Union[str, Step]],
    available: Optional[Dict[str, Type[Step]]],
) -> List[Step]:
    registry = available if available is not None else all_steps()
    resolved: List[Step] = []
    for entry in steps:
        if isinstance(entry, Step):
            resolved.append(entry)
        else:
            if entry not in registry:
                raise StepExecutionError(entry, "unknown step")
            resolved.append(registry[entry]())
    return resolved


def _step_name(step: Step) -> str:
    return step.name or step.__class__.__name__


def _warn_missing_requires(ctx, step: Step, step_name: str) -> None:
    registry = getattr(ctx, "artifact_registry", None)
    if registry is None:
        return
    missing = [a for a in step.requires if not registry.has(a)]
    if missing:
        log_warning(
            f"⚠️  {step_name} declares required artifacts not in this run: "
            f"{', '.join(missing)} (resolving from disk)"
        )


def _finish_step(
    subscribers: Sequence[Subscriber],
    results: RunResult,
    run_name: str,
    step: Step,
    step_start: float,
    error: Optional[str] = None,
) -> float:
    duration = time.time() - step_start
    status: RunStatus = "failed" if error else "success"
    results.steps.append(
        StepResult(name=_step_name(step), status=status, duration=duration)
    )
    _emit(
        subscribers,
        StepFinished(
            run=run_name,
            step=_step_name(step),
            phase=step.phase,
            status=status,
            duration=duration,
            error=error,
        ),
    )
    return duration


def _log_run_success(start: float) -> None:
    duration = time.time() - start
    mins = int(duration / 60)
    secs = int(duration % 60)
    log_info("\n" + "=" * 70)
    log_success(f"✅ Pipeline completed successfully in {mins}m {secs}s")
    log_info("=" * 70)


def _emit(subscribers: Sequence[Subscriber], event: Event) -> None:
    for subscriber in subscribers:
        try:
            subscriber(event)
        except Exception as e:
            log_warning(f"Notification subscriber failed for {type(event).__name__}: {e}")
