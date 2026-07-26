#!/usr/bin/env python3
"""Build CLI - Modular build system for BrowserOS"""

import os
import re
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Callable, List, Optional, Tuple

import typer

from ..core.context import Context
from ..core.checkout_lock import CheckoutLockError, ChromiumCheckoutLock
from ..lib.paths import get_package_root
from ..core.pipeline import validate_pipeline, show_available_modules
from ..core.planner import (
    Profile,
    Switches,
    VALID_ARCHITECTURES,
    load_profile,
    plan_runs,
    preflight,
    required_env,
    slice_runs_from,
)
from ..core.resolver import resolve_config, resolve_pipeline
from ..lib.notify import slack_subscriber
from ..core.runner import StepExecutionError, run as run_pipeline
from ..core.step import (
    all_steps,
    phase_steps,
)
from ..lib.utils import (
    get_platform,
    get_platform_arch,
    log_error,
    log_info,
    log_warning,
    IS_WINDOWS,
)

# All of these derive from step registration metadata (core/step.py);
# pipeline order within a phase comes from steps/__init__.py import order.
AVAILABLE_MODULES = all_steps()

EXECUTION_ORDER = [
    (phase, phase_steps(phase))
    for phase in ("setup", "prep", "build", "sign", "package", "upload")
]


def main(
    preset: Optional[str] = typer.Option(
        None,
        "--preset",
        help="Pipeline preset: release or debug (planner composes the steps)",
    ),
    profile: Optional[Path] = typer.Option(
        None,
        "--profile",
        help="Profile file of saved switches (bos_build/profiles/*.yaml or a path)",
    ),
    product: Optional[str] = typer.Option(
        None,
        "--product",
        "-p",
        help="Product to build (browseros, browserclaw)",
    ),
    modules: Optional[str] = typer.Option(
        None,
        "--modules",
        "-m",
        help="Comma-separated list of modules to run",
    ),
    list_modules: bool = typer.Option(
        False,
        "--list",
        "-l",
        help="List all available modules and exit",
    ),
    # Pipeline phase flags (auto-ordered execution)
    setup: bool = typer.Option(
        False,
        "--setup",
        help="Run setup phase (clean, git_setup, sparkle_setup/winsparkle_setup)",
    ),
    prep: bool = typer.Option(
        False,
        "--prep",
        help="Run prep phase (resources, chromium_replace, string_replaces, patches, configure)",
    ),
    build: bool = typer.Option(
        False,
        "--build",
        help="Run build phase (compile)",
    ),
    package: bool = typer.Option(
        False,
        "--package",
        help="Run package phase (platform-specific: package_macos/windows/linux)",
    ),
    # Tri-state toggles: phase flag when used alone (--sign / --upload);
    # switch override in preset/profile mode (--no-sign / --no-upload).
    sign: Optional[bool] = typer.Option(
        None,
        "--sign/--no-sign",
        help="Phase mode: run sign phase. Preset mode: toggle signing",
    ),
    upload: Optional[bool] = typer.Option(
        None,
        "--upload/--no-upload",
        help="Phase mode: run upload phase. Preset mode: toggle upload",
    ),
    clean: Optional[bool] = typer.Option(
        None,
        "--clean/--no-clean",
        help="Preset mode: toggle the clean step",
    ),
    provision: Optional[str] = typer.Option(
        None,
        "--provision",
        help="Preset mode: chromium provisioning (none, full, shallow)",
    ),
    download: Optional[bool] = typer.Option(
        None,
        "--download/--no-download",
        help="Preset mode: toggle downloading server resources from R2",
    ),
    skip: Optional[str] = typer.Option(
        None,
        "--skip",
        help="Preset mode: comma-separated steps to subtract from the composed "
        "plan (unions with the profile's skip:)",
    ),
    from_: Optional[str] = typer.Option(
        None,
        "--from",
        help="Preset mode: resume from this step (slices the composed plan)",
    ),
    show_plan: bool = typer.Option(
        False,
        "--show-plan",
        help="Print the composed step list + required env vars and exit "
        "(needs no chromium checkout)",
    ),
    # Global options
    arch: Optional[str] = typer.Option(
        None,
        "--arch",
        "-a",
        help="Architecture (arm64, x64, universal)",
    ),
    build_type: Optional[str] = typer.Option(
        None,
        "--build-type",
        "-t",
        help="Build type for --modules/phase mode (debug or release)",
    ),
    chromium_src: Optional[Path] = typer.Option(
        None,
        "--chromium-src",
        "-S",
        help="Path to Chromium source directory",
    ),
    lock_wait: bool = typer.Option(
        False,
        "--lock-wait",
        help="Wait for another BrowserOS build using the same Chromium checkout "
        "instead of failing fast",
    ),
    gn_arg: Optional[List[str]] = typer.Option(
        None,
        "--gn-arg",
        help="Append a GN arg to args.gn after all flags (key=value, repeatable). "
        "Written last, so it wins. GN syntax applies: bools/ints are bare, "
        "strings need embedded quotes (--gn-arg 'target_cpu=\"arm64\"'). "
        "Per-invocation only — never persisted; use a flags file for durable "
        "changes.",
    ),
):
    """BrowserOS Build System - Modular pipeline executor

    Build BrowserOS with a preset (planner-composed), phase flags, or
    explicit modules.

    \b
    Presets (Recommended - one pipeline definition, switches select):
      browseros build --preset release --product browseros --arch arm64
      browseros build --preset release --product browserclaw --no-upload
      browseros build --profile nightly-ci --arch x64
      browseros build --preset debug

    \b
    Plan Visibility & Subtraction:
      browseros build --preset release --show-plan
      browseros build --preset release --skip upload,series_patches
      browseros build --preset release --from sign_macos

    \b
    Phase Flags (Auto-Ordered):
      browseros build --setup --build --sign --package
      browseros build --build --sign           # Skip setup

    \b
    Explicit Modules (Power Users):
      browseros build --modules clean,compile,sign_macos

    \b
    List Available:
      browseros build --list                   # Show all modules and phases
    """

    if list_modules:
        show_available_modules(AVAILABLE_MODULES)
        return

    try:
        extra_gn_args = _parse_gn_args(gn_arg)
    except ValueError as e:
        log_error(str(e))
        raise typer.Exit(1)

    has_preset = preset is not None or profile is not None
    has_modules = modules is not None
    # --sign/--upload given affirmatively without a preset are phase flags
    phase_sign = sign is True and not has_preset
    phase_upload = upload is True and not has_preset
    has_flags = any([setup, prep, build, package, phase_sign, phase_upload])

    options_provided = sum([has_preset, has_modules, has_flags])

    if options_provided == 0:
        typer.echo(
            "Error: Specify --preset/--profile, --modules, or phase flags (--setup, --build, etc.)\n"
        )
        typer.echo("Use --help for usage information")
        typer.echo("Use --list to see available modules")
        raise typer.Exit(1)

    if options_provided > 1:
        log_error("Specify only ONE of: --preset/--profile, --modules, or phase flags")
        log_error("Examples:")
        log_error("  browseros build --preset release --product browserclaw")
        log_error("  browseros build --setup --build --sign")
        log_error("  browseros build --modules clean,compile")
        raise typer.Exit(1)

    if (skip is not None or from_ is not None) and not has_preset:
        log_error(
            "--skip/--from apply to preset/profile mode — they subtract from "
            "the planner-composed pipeline; edit --modules lists directly"
        )
        raise typer.Exit(1)

    # Plan projection happens before the banner and before anything touches
    # the chromium checkout, so --show-plan works on a machine without one.
    if has_preset:
        projection = _resolve_preset(
            preset=preset,
            profile=profile,
            product=product,
            arch=arch,
            clean=clean,
            provision=provision,
            download=download,
            sign=sign,
            upload=upload,
            build_type=build_type,
            skip=skip,
            from_=from_,
            chromium_src=chromium_src,
            extra_gn_args=extra_gn_args,
        )
        if show_plan:
            _print_plan(projection)
            return
    else:
        cli_args = {
            "chromium_src": chromium_src,
            "arch": arch,
            "build_type": build_type,
            "product": product,
            "modules": modules,
            "setup": setup,
            "prep": prep,
            "build": build,
            "sign": phase_sign,
            "package": package,
            "upload": phase_upload,
            "extra_gn_args": extra_gn_args,
        }
        try:
            pipeline = resolve_pipeline(
                cli_args, execution_order=EXECUTION_ORDER, quiet=show_plan
            )
        except ValueError as e:
            log_error(str(e))
            raise typer.Exit(1)
        if show_plan:
            validate_pipeline(pipeline, AVAILABLE_MODULES)
            from ..lib.env import EnvConfig

            label_arch = arch or EnvConfig().arch or get_platform_arch()
            if label_arch not in VALID_ARCHITECTURES:
                log_error(
                    f"Invalid architecture '{label_arch}'. "
                    f"Valid: {', '.join(VALID_ARCHITECTURES)}"
                )
                raise typer.Exit(1)
            header = ["Direct pipeline (--modules/phase flags)"]
            if extra_gn_args:
                header.append(f"GN arg overrides: {', '.join(extra_gn_args)}")
            _print_plan(
                _PlanProjection(
                    header=header,
                    arch_plans=[(label_arch, pipeline)],
                )
            )
            return

    log_info("🚀 BrowserOS Build System")
    log_info("=" * 70)

    root_dir = get_package_root()

    if has_preset:
        runs = projection.build_runs()
    else:
        try:
            arch_ctxs = resolve_config(cli_args)
        except ValueError as e:
            log_error(str(e))
            raise typer.Exit(1)
        runs = [(ctx, pipeline) for ctx in arch_ctxs]

    try:
        _execute_runs_with_checkout_lock(
            runs,
            lock_wait=lock_wait,
            has_flags=has_flags,
            prep=prep,
            root_dir=root_dir,
        )
    except CheckoutLockError as e:
        log_error(str(e))
        raise typer.Exit(1)


def _execute_runs_with_checkout_lock(
    runs: List[Tuple[Context, List[str]]],
    *,
    lock_wait: bool,
    has_flags: bool,
    prep: bool,
    root_dir: Path,
) -> None:
    if not runs:
        raise typer.Exit(1)

    summary_ctx = runs[0][0]
    log_info(f"🔒 Acquiring Chromium checkout lock: {summary_ctx.chromium_src}")
    with ChromiumCheckoutLock(
        summary_ctx.chromium_src,
        product=summary_ctx.product.id,
        wait=lock_wait,
    ) as checkout_lock:
        log_info(f"🔒 Chromium checkout lock acquired: {checkout_lock.lock_path}")

        _execute_runs(
            runs,
            has_flags=has_flags,
            prep=prep,
            root_dir=root_dir,
        )


def _execute_runs(
    runs: List[Tuple[Context, List[str]]],
    *,
    has_flags: bool,
    prep: bool,
    root_dir: Path,
) -> None:
    if has_flags:
        log_info("\n📋 Execution Plan (auto-ordered):")
        log_info("-" * 70)
        if prep:
            log_warning(
                "⚠️  --prep does NOT apply series_patches. Run 'browseros build -m series_patches' separately if needed."
            )
        log_info(f"  Pipeline: {' → '.join(runs[0][1])}")
        log_info("-" * 70)

    for _, run_steps in runs:
        validate_pipeline(run_steps, AVAILABLE_MODULES)

    # Whole-pipeline static preflight (env from step metadata, platform,
    # per-step static checks) for EVERY arch before any run starts — a
    # misconfigured second arch must not surface after hours of arch one.
    try:
        for run_ctx, run_steps in runs:
            preflight(run_steps, ctx=run_ctx)
    except ValueError as e:
        log_error(str(e))
        raise typer.Exit(1)

    if IS_WINDOWS():
        os.environ["DEPOT_TOOLS_WIN_TOOLCHAIN"] = "0"
        log_info("Set DEPOT_TOOLS_WIN_TOOLCHAIN=0 for Windows build")

    # Print build summary using the first context — versions and paths
    # are identical across per-arch contexts.
    summary_ctx = runs[0][0]
    log_info(f"📍 Root: {root_dir}")
    log_info(f"📍 Chromium: {summary_ctx.chromium_src}")
    if len(runs) > 1:
        log_info(
            f"📍 Architectures: {[c.architecture for c, _ in runs]} (multi-arch loop)"
        )
    else:
        log_info(f"📍 Architecture: {summary_ctx.architecture}")
    log_info(f"📍 Product: {summary_ctx.product.id}")
    log_info(f"📍 Build type: {summary_ctx.build_type}")
    if summary_ctx.extra_gn_args:
        log_info(f"📍 GN arg overrides: {', '.join(summary_ctx.extra_gn_args)}")
        if not any("configure" in run_steps for _, run_steps in runs):
            log_warning(
                "⚠️  --gn-arg has no effect: no run in this plan includes the "
                "configure step, so args.gn is reused as-is"
            )
    log_info(f"📍 Semantic version: {summary_ctx.semantic_version}")
    log_info(f"📍 Chromium version: {summary_ctx.chromium_version}")
    log_info(f"📍 Build offset: {summary_ctx.browseros_build_offset}")
    if len(runs) > 1:
        # Runs may be heterogeneous (universal), so show each run's steps
        for run_ctx, run_steps in runs:
            log_info(f"📍 Pipeline[{run_ctx.architecture}]: {' → '.join(run_steps)}")
    else:
        log_info(f"📍 Pipeline: {' → '.join(runs[0][1])}")
    log_info("=" * 70)

    # Execute once per architecture. Steps see a normal single-arch ctx;
    # only this loop knows about multi-arch.
    multi_arch = len(runs) > 1
    for i, (arch_ctx, run_steps) in enumerate(runs, start=1):
        if multi_arch:
            log_info("\n" + "#" * 70)
            log_info(f"# Architecture {i}/{len(runs)}: {arch_ctx.architecture}")
            log_info(f"# Output: {arch_ctx.out_dir}")
            log_info("#" * 70)

        try:
            run_pipeline(
                arch_ctx,
                run_steps,
                name="build",
                subscribers=(slack_subscriber(arch_ctx),),
                available=AVAILABLE_MODULES,
            )
        except StepExecutionError as e:
            log_error(str(e))
            raise typer.Exit(1)
        except KeyboardInterrupt:
            raise typer.Exit(130)


def _display_only_runs() -> List[Tuple[Context, List[str]]]:
    raise RuntimeError("plan projection is display-only; no run builder attached")


@dataclass(frozen=True)
class _PlanProjection:
    """Composed plan, printable without a chromium checkout.

    build_runs defers Context construction (and thus chromium_src
    resolution) so --show-plan can exit before either happens.
    """

    header: List[str]
    arch_plans: List[Tuple[str, List[str]]]
    build_runs: Callable[[], List[Tuple[Context, List[str]]]] = _display_only_runs


def _resolve_preset(
    *,
    preset: Optional[str],
    profile: Optional[Path],
    product: Optional[str],
    arch: Optional[str],
    clean: Optional[bool],
    provision: Optional[str],
    download: Optional[bool],
    sign: Optional[bool],
    upload: Optional[bool],
    build_type: Optional[str],
    skip: Optional[str],
    from_: Optional[str],
    chromium_src: Optional[Path],
    extra_gn_args: Tuple[str, ...] = (),
) -> _PlanProjection:
    """Resolve preset/profile + CLI overrides into a plan projection.

    Precedence: CLI > profile > preset defaults, except --skip which
    UNIONS with the profile's skip: (both are commented-out sets). A
    profile carrying modules: routes to the DIRECT-mode machinery.
    """
    try:
        prof = (
            load_profile(_resolve_profile_path(profile))
            if profile
            else Profile(Switches())
        )
        if prof.modules is not None:
            return _resolve_modules_profile(
                prof,
                preset=preset,
                product=product,
                arch=arch,
                clean=clean,
                provision=provision,
                download=download,
                sign=sign,
                upload=upload,
                build_type=build_type,
                skip=skip,
                from_=from_,
                chromium_src=chromium_src,
                extra_gn_args=extra_gn_args,
            )
        if build_type is not None:
            raise ValueError(
                "--build-type is owned by the preset (release/debug); drop it"
            )
        switches = prof.switches
        overrides = {}
        if preset is not None:
            overrides["preset"] = preset
        if product is not None:
            overrides["product"] = product
        if arch is not None:
            overrides["architectures"] = (arch,)
        if clean is not None:
            overrides["clean"] = clean
        if provision is not None:
            overrides["provision"] = provision
        if download is not None:
            overrides["download"] = download
        if sign is not None:
            overrides["sign"] = sign
        if upload is not None:
            overrides["upload"] = upload
        switches = replace(switches, **overrides)
        cli_skip = _parse_steps_csv(skip)
        if cli_skip:
            switches = replace(
                switches, skip=tuple(dict.fromkeys((*switches.skip, *cli_skip)))
            )
        switches = switches.resolved()

        # Runs execute sequentially (universal is three runs on one tree),
        # so --from resumes the run timeline, not each run.
        arch_plans = plan_runs(switches)
        if from_ is not None:
            arch_plans = slice_runs_from(arch_plans, from_)

        header = [
            f"Preset plan: preset={switches.preset} product={switches.product} "
            f"platform={get_platform()}",
            f"Switches: clean={switches.clean} provision={switches.provision} "
            f"download={switches.download} sign={switches.sign} "
            f"upload={switches.upload} "
            f"bundle_local_extensions={switches.bundle_local_extensions}",
        ]
        if switches.skip:
            header.append(f"Skip: {', '.join(switches.skip)}")
        if from_ is not None:
            header.append(f"From: {from_}")
        if extra_gn_args:
            header.append(f"GN arg overrides: {', '.join(extra_gn_args)}")

        def build_runs() -> List[Tuple[Context, List[str]]]:
            try:
                for run_arch, steps in arch_plans:
                    if not steps:
                        raise ValueError(
                            f"plan for {run_arch} is empty after skip — nothing to run"
                        )
                # Shallow provisioning creates the checkout itself, so the
                # src dir may not exist yet on a fresh runner.
                src = _resolve_chromium_src(
                    chromium_src, allow_missing=switches.provision == "shallow"
                )
                log_info(
                    f"✓ PRESET MODE: preset={switches.preset} "
                    f"product={switches.product}"
                )
                log_info(
                    f"✓ PRESET MODE: clean={switches.clean} "
                    f"provision={switches.provision} download={switches.download} "
                    f"sign={switches.sign} upload={switches.upload} "
                    f"bundle_local_extensions={switches.bundle_local_extensions}"
                )
                if switches.skip:
                    log_info(f"✓ PRESET MODE: skip={','.join(switches.skip)}")
                if from_ is not None:
                    log_info(f"✓ PRESET MODE: from={from_}")
                if extra_gn_args:
                    log_info(
                        f"✓ PRESET MODE: gn-arg overrides={','.join(extra_gn_args)}"
                    )
                return [
                    (
                        Context(
                            chromium_src=src,
                            architecture=run_arch,
                            plan_architectures=tuple(switches.architectures),
                            build_type=switches.build_type,
                            product=switches.product,
                            extra_gn_args=extra_gn_args,
                            bundle_local_extensions=switches.bundle_local_extensions,
                        ),
                        steps,
                    )
                    for run_arch, steps in arch_plans
                ]
            except ValueError as e:
                log_error(str(e))
                raise typer.Exit(1)

        return _PlanProjection(header, arch_plans, build_runs)
    except ValueError as e:
        log_error(str(e))
        raise typer.Exit(1)


def _resolve_modules_profile(
    prof: Profile,
    *,
    preset: Optional[str],
    product: Optional[str],
    arch: Optional[str],
    clean: Optional[bool],
    provision: Optional[str],
    download: Optional[bool],
    sign: Optional[bool],
    upload: Optional[bool],
    build_type: Optional[str],
    skip: Optional[str],
    from_: Optional[str],
    chromium_src: Optional[Path],
    extra_gn_args: Tuple[str, ...] = (),
) -> _PlanProjection:
    """Project a modules: profile through the DIRECT-mode machinery.

    The profile owns its pipeline, so planner flags (including --skip and
    --from) are rejected; only --product/--arch/--build-type override the
    file (CLI > profile).
    """
    rejected = {
        "--preset": preset,
        "--clean/--no-clean": clean,
        "--provision": provision,
        "--download/--no-download": download,
        "--sign/--no-sign": sign,
        "--upload/--no-upload": upload,
        "--skip": skip,
        "--from": from_,
    }
    given = sorted(flag for flag, value in rejected.items() if value is not None)
    if given:
        raise ValueError(
            f"{', '.join(given)}: planner flags do not combine with a modules "
            "profile — it owns its pipeline; edit the modules list instead"
        )

    from ..lib.env import EnvConfig

    steps = list(prof.modules or ())
    validate_pipeline(steps, AVAILABLE_MODULES)
    eff_product = product or prof.switches.product
    profile_arch = (
        prof.switches.architectures[0] if prof.switches.architectures else None
    )
    # Fully resolve arch at projection time (CLI > profile > env >
    # platform) so the printed label can never disagree with the run.
    eff_arch = arch or profile_arch or EnvConfig().arch or get_platform_arch()
    if eff_arch not in VALID_ARCHITECTURES:
        raise ValueError(
            f"Invalid architecture '{eff_arch}'. "
            f"Valid: {', '.join(VALID_ARCHITECTURES)}"
        )
    eff_build_type = build_type or prof.build_type or "debug"
    if eff_build_type not in ("debug", "release"):
        raise ValueError(
            f"Invalid build type '{eff_build_type}'. Valid: debug, release"
        )

    header = [
        "Modules profile (enumerated pipeline — you own this list)",
        f"product={eff_product} arch={eff_arch} build_type={eff_build_type}",
    ]
    if extra_gn_args:
        header.append(f"GN arg overrides: {', '.join(extra_gn_args)}")

    def build_runs() -> List[Tuple[Context, List[str]]]:
        try:
            arch_ctxs = resolve_config(
                {
                    "chromium_src": chromium_src,
                    "arch": eff_arch,
                    "build_type": eff_build_type,
                    "product": eff_product,
                    "extra_gn_args": extra_gn_args,
                }
            )
        except ValueError as e:
            log_error(str(e))
            raise typer.Exit(1)
        return [(ctx, steps) for ctx in arch_ctxs]

    return _PlanProjection(header, [(eff_arch, steps)], build_runs)


def _parse_steps_csv(value: Optional[str]) -> Tuple[str, ...]:
    if not value:
        return ()
    return tuple(s.strip() for s in value.split(",") if s.strip())


_GN_ARG_RE = re.compile(r"^[a-zA-Z_][a-zA-Z0-9_]*=.+\Z")


def _parse_gn_args(values: Optional[List[str]]) -> Tuple[str, ...]:
    """Validate --gn-arg values as GN key=value; values stay verbatim."""
    args = tuple(values or ())
    for value in args:
        if not _GN_ARG_RE.match(value):
            raise ValueError(
                f"Invalid --gn-arg '{value}': expected key=value, e.g. "
                'symbol_level=2 or target_cpu="arm64"'
            )
    return args


def _print_plan(projection: _PlanProjection) -> None:
    """Print the composed steps + required env; env values never shown."""
    for line in projection.header:
        typer.echo(line)
    for arch_name, steps in projection.arch_plans:
        typer.echo("")
        typer.echo(f"{arch_name} ({len(steps)} steps):")
        for i, name in enumerate(steps, start=1):
            typer.echo(f"  {i:2}. {name}")
        env_vars = required_env(steps)
        typer.echo("")
        if not env_vars:
            typer.echo(f"Required env ({arch_name}): none")
            continue
        typer.echo(f"Required env ({arch_name}):")
        for var in env_vars:
            marker = "✓ set" if os.environ.get(var) else "✗ MISSING"
            typer.echo(f"  {var}  {marker}")


def _resolve_profile_path(profile: Path) -> Path:
    """Accept a bare profile name (nightly-ci) or a path to a yaml file."""
    if profile.exists():
        return profile
    candidate = get_package_root() / "bos_build" / "profiles" / f"{profile.name}.yaml"
    if candidate.exists():
        return candidate
    raise ValueError(f"Profile not found: {profile} (also tried {candidate})")


def _resolve_chromium_src(
    chromium_src: Optional[Path], allow_missing: bool = False
) -> Path:
    """chromium_src: CLI > CHROMIUM_SRC env > error (same as direct mode)."""
    from ..lib.env import EnvConfig

    src = chromium_src or EnvConfig().chromium_src
    if not src:
        raise ValueError(
            "chromium_src required!\n"
            "Provide via one of:\n"
            "  --chromium-src PATH\n"
            "  CHROMIUM_SRC environment variable"
        )
    src = Path(src)
    if not src.exists() and not allow_missing:
        raise ValueError(f"chromium_src does not exist: {src}")
    return src
