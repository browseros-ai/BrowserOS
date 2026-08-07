#!/usr/bin/env python3
"""
Configuration resolver for DIRECT mode (--modules / phase flags).

Precedence: CLI args > Environment variables (CHROMIUM_SRC, ARCH) >
Hardcoded defaults.

Preset/profile mode (the old CONFIG mode's replacement) is resolved by
core.planner via cli/build.py — release YAML module lists are gone.
"""

from pathlib import Path
from typing import Optional, List, Dict, Any, Tuple

from .context import Context
from ..lib.env import EnvConfig
from .products import get_product_descriptor
from ..lib.utils import get_platform_arch, log_info

VALID_ARCHITECTURES = {"x64", "arm64", "universal"}


def resolve_config(cli_args: Dict[str, Any]) -> List[Context]:
    """Resolve DIRECT-mode build configuration: CLI > Env > Defaults.

    Returns:
        Single-element list with the resolved Context (DIRECT mode is
        always single-arch; CLI --arch is a scalar). List-shaped for
        symmetry with preset-mode multi-arch runs.

    Raises:
        ValueError: If chromium_src not provided or invalid

    Note:
        root_dir is always computed from package location via
        get_package_root(), never from config or cwd.
    """
    env = EnvConfig()

    # chromium_src: CLI > Env > Error
    chromium_src = cli_args.get("chromium_src") or env.chromium_src
    if not chromium_src:
        raise ValueError(
            "DIRECT MODE: chromium_src required!\n"
            "Provide via one of:\n"
            "  --chromium-src PATH\n"
            "  CHROMIUM_SRC environment variable"
        )

    chromium_src = Path(chromium_src)

    if not chromium_src.exists():
        raise ValueError(
            f"DIRECT MODE: chromium_src does not exist: {chromium_src}\n"
            f"Expected directory with Chromium source code"
        )

    # architecture: CLI > Env > Platform default
    architecture = cli_args.get("arch") or env.arch
    if not architecture:
        architecture = get_platform_arch()
        log_info(f"DIRECT MODE: Using platform default architecture: {architecture}")

    if architecture not in VALID_ARCHITECTURES:
        raise ValueError(
            f"DIRECT MODE: invalid architecture '{architecture}'. "
            f"Valid: {sorted(VALID_ARCHITECTURES)}"
        )

    # build_type: CLI > Default
    build_type = cli_args.get("build_type") or "debug"

    product = get_product_descriptor(cli_args.get("product"))

    extra_gn_args = tuple(cli_args.get("extra_gn_args") or ())

    log_info(f"✓ DIRECT MODE: chromium_src={chromium_src} (cli/env)")
    log_info(f"✓ DIRECT MODE: architecture={architecture} (cli/env/default)")
    log_info(f"✓ DIRECT MODE: build_type={build_type} (cli/default)")
    log_info(f"✓ DIRECT MODE: product={product.id} (cli/default)")
    if extra_gn_args:
        log_info(f"✓ DIRECT MODE: gn-arg overrides={','.join(extra_gn_args)} (cli)")

    return [
        Context(
            chromium_src=chromium_src,
            architecture=architecture,
            plan_architectures=(architecture,),
            build_type=build_type,
            product=product,
            extra_gn_args=extra_gn_args,
        )
    ]


def resolve_pipeline(
    cli_args: Dict[str, Any],
    execution_order: Optional[List[Tuple[str, List[str]]]] = None,
    quiet: bool = False,
) -> List[str]:
    """Resolve DIRECT-mode pipeline from --modules or phase flags.

    Args:
        cli_args: CLI arguments dictionary
        execution_order: Phase execution order (required for flag mode)
        quiet: Suppress resolution logging (--show-plan projections)

    Returns:
        List of module names in execution order

    Raises:
        ValueError: If no pipeline specified or conflicting modes
    """
    has_modules = cli_args.get("modules") is not None
    has_flags = _has_phase_flags(cli_args)

    if not has_modules and not has_flags:
        raise ValueError(
            "DIRECT MODE: No pipeline specified!\n"
            "Use one of:\n"
            "  --modules clean,compile,...\n"
            "  --setup --build --sign  (phase flags)"
        )

    if has_modules and has_flags:
        raise ValueError(
            "DIRECT MODE: Cannot use both --modules and phase flags!\n"
            "Choose one approach."
        )

    if has_modules:
        modules_str = cli_args["modules"]
        pipeline = [m.strip() for m in modules_str.split(",")]
        if not quiet:
            log_info(f"✓ DIRECT MODE: pipeline={pipeline} (--modules)")
        return pipeline

    if execution_order is None:
        raise ValueError(
            "DIRECT MODE: execution_order required for phase flag resolution"
        )
    pipeline = _build_pipeline_from_flags(cli_args, execution_order)
    if not quiet:
        log_info(f"✓ DIRECT MODE: pipeline={pipeline} (phase flags)")
    return pipeline


def _has_phase_flags(cli_args: Dict[str, Any]) -> bool:
    """Check if any phase flags are set."""
    phase_flags = ["setup", "prep", "build", "sign", "package", "upload"]
    return any(cli_args.get(flag, False) for flag in phase_flags)


def _build_pipeline_from_flags(
    cli_args: Dict[str, Any],
    execution_order: List[Tuple[str, List[str]]],
) -> List[str]:
    """Build pipeline from phase flags with fixed execution order."""
    enabled_phases = {
        "setup": cli_args.get("setup", False),
        "prep": cli_args.get("prep", False),
        "build": cli_args.get("build", False),
        "sign": cli_args.get("sign", False),
        "package": cli_args.get("package", False),
        "upload": cli_args.get("upload", False),
    }

    pipeline = []
    for phase_name, phase_modules in execution_order:
        if enabled_phases.get(phase_name, False):
            pipeline.extend(phase_modules)

    return pipeline
