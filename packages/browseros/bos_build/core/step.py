#!/usr/bin/env python3
"""Step contract and registry for the build pipeline.

A step is one discrete pipeline unit (clean, compile, sign_macos, ...).
Classes register with the @step decorator, which attaches metadata the
CLI and planner derive everything from: available steps and per-platform
phase ordering. Within a phase, order is
registration order — bos_build/steps/__init__.py imports step modules
in canonical pipeline order, so that file is the single place ordering
lives.
"""

from typing import Dict, List, Optional, Tuple, Type

from ..lib.utils import get_platform

# Canonical phase order. "source" is reserved for chromium provisioning.
PHASES: Tuple[str, ...] = (
    "source",
    "setup",
    "prep",
    "build",
    "sign",
    "package",
    "upload",
)


class ValidationError(Exception):
    """
    Raised when step validation fails

    This exception is raised by the validate() method when a step cannot execute
    due to missing requirements, platform incompatibility, or invalid configuration.
    The build pipeline stops immediately when ValidationError is raised.
    """

    pass


class Step:
    """
    Base class for all build steps

    Each step represents a discrete unit in the build pipeline (e.g., clean,
    compile, sign). Steps are self-contained and declare their requirements
    and outputs explicitly.

    Registration metadata (set by the @step decorator):
        name: Registry name (e.g. "sign_macos")
        phase: One of PHASES
        platforms: Platforms the step applies to; None = all
        env: Environment variable names the step needs (preflighted)
        optional: Excluded from phase-flag/preset expansion unless
            explicitly requested (e.g. series_patches, merge_universal)

    Contract attributes (plain class attributes):
        produces: Artifact names this step creates (e.g. ["signed_app"])
        requires: Artifact names this step needs (e.g. ["built_app"])
        description: Human-readable description for --list output

    Methods:
        validate(context): Check if the step can run, raise ValidationError if not
        execute(context): Execute the step's main task
    """

    # Registration metadata (set by @step; empty for unregistered helpers)
    name: str = ""
    phase: str = ""
    platforms: Optional[Tuple[str, ...]] = None
    env: Tuple[str, ...] = ()
    optional: bool = False

    # Contract metadata
    produces: List[str] = []
    requires: List[str] = []
    description: str = "No description provided"

    def preflight(self, context) -> None:
        """
        Static plan-time checks, run for the WHOLE pipeline before step 1
        executes (a misconfigured nightly fails in seconds, not at hour 3).

        Only check state that exists before the run starts (tools on PATH,
        static files, SDK versions). Env vars and platform come free from
        the env=/platforms= metadata — don't recheck them here. State
        produced mid-run (the built app, artifacts) belongs in validate().
        Raise ValidationError on failure.
        """

    def validate(self, context) -> None:
        """
        Validate that this step can run successfully

        Runs just-in-time before execute() — the right place for dynamic
        state produced earlier in the run (e.g. sign checks the app that
        compile just built). Static env/platform checks belong to
        metadata + preflight. The pipeline stops on ValidationError.
        """
        raise NotImplementedError(
            f"{self.__class__.__name__} must implement validate()"
        )

    def execute(self, context) -> None:
        """
        Execute the step's main task

        Log progress, register produced artifacts on the context, raise on
        failure (stops the pipeline). Only called after validate() succeeds.
        Steps should be idempotent where possible.
        """
        raise NotImplementedError(
            f"{self.__class__.__name__} must implement execute()"
        )

    def applies_to(self, platform: str) -> bool:
        """Whether this step runs on the given platform."""
        return self.platforms is None or platform in self.platforms



# Insertion-ordered: registration order within a phase IS pipeline order.
_REGISTRY: Dict[str, Type[Step]] = {}


def step(
    name: str,
    *,
    phase: str,
    platforms: Optional[Tuple[str, ...]] = None,
    env: Tuple[str, ...] = (),
    optional: bool = False,
):
    """Register a Step subclass in the pipeline registry."""
    if phase not in PHASES:
        raise ValueError(f"Unknown phase '{phase}' for step '{name}'. Valid: {PHASES}")

    def decorator(cls: Type[Step]) -> Type[Step]:
        if not issubclass(cls, Step):
            raise TypeError(f"@step target {cls.__name__} must subclass Step")
        if name in _REGISTRY:
            raise ValueError(
                f"Duplicate step name '{name}' "
                f"({_REGISTRY[name].__name__} vs {cls.__name__})"
            )
        cls.name = name
        cls.phase = phase
        cls.platforms = platforms
        cls.env = env
        cls.optional = optional
        _REGISTRY[name] = cls
        return cls

    return decorator


def all_steps() -> Dict[str, Type[Step]]:
    """All registered steps by name, in registration order."""
    _ensure_loaded()
    return dict(_REGISTRY)


def get_step(name: str) -> Type[Step]:
    """Look up a registered step class by name."""
    _ensure_loaded()
    return _REGISTRY[name]


def phase_steps(
    phase: str,
    platform: Optional[str] = None,
    include_optional: bool = False,
) -> List[str]:
    """Step names for a phase, platform-filtered, in pipeline order."""
    _ensure_loaded()
    platform = platform or get_platform()
    return [
        name
        for name, cls in _REGISTRY.items()
        if cls.phase == phase
        and (include_optional or not cls.optional)
        and (cls.platforms is None or platform in cls.platforms)
    ]


def _ensure_loaded() -> None:
    """Import the steps package so decorators have run.

    Deferred (not module-level) to avoid a core → steps import cycle;
    steps modules import core.step for the decorator itself.
    """
    from importlib import import_module

    import_module("bos_build.steps")
