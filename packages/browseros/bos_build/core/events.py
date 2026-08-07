#!/usr/bin/env python3
"""Lifecycle events emitted by the pipeline runner.

Subscribers (Slack, logging, tests) receive every event; RunFinished is
emitted exactly once per run — success, failure, or interrupt — which is
what makes end-of-run notifications reliable.
"""

from dataclasses import dataclass
from typing import Callable, Literal, Optional, Tuple, Union

RunStatus = Literal["success", "failed", "interrupted"]


@dataclass(frozen=True)
class RunStarted:
    run: str
    steps: Tuple[str, ...]


@dataclass(frozen=True)
class StepStarted:
    run: str
    step: str
    phase: str


@dataclass(frozen=True)
class StepFinished:
    run: str
    step: str
    phase: str
    status: RunStatus
    duration: float
    error: Optional[str] = None


@dataclass(frozen=True)
class RunFinished:
    run: str
    status: RunStatus
    duration: float
    error: Optional[str] = None


Event = Union[RunStarted, StepStarted, StepFinished, RunFinished]
Subscriber = Callable[[Event], None]
