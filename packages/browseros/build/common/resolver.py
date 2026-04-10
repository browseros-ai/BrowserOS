#!/usr/bin/env python3
"""
Configuration resolver - single source of truth for all config resolution

Two mutually exclusive modes:
1. CONFIG mode (--config FILE): YAML controls everything
2. DIRECT mode (no --config): CLI args > Env > Defaults

Precedence (CONFIG mode):
  - YAML (authoritative)
  - Env vars (only for secrets/credentials via EnvConfig)
  - Error if required fields missing

Precedence (DIRECT mode):
  - CLI args (explicit, Typer defaults must be None)
  - Environment variables (CHROMIUM_SRC, ARCH)
  - Hardcoded defaults

This centralizes ALL configuration resolution in one place.
"""

from pathlib import Path
from typing import Optional, List, Dict, Any, Tuple

from .context import Context
from .env import EnvConfig
from .utils import get_platform_arch, log_info

VALID_ARCHITECTURES = {"x64", "arm64", "universal"}


def resolve_config(
    cli_args: Dict[str, Any],
    yaml_config: Optional[Dict[str, Any]] = None,
) -> List[Context]:
    """Resolve build configuration - single entry point.

    Args:
        cli_args: Dictionary of CLI arguments (all values should be None if not provided)
        yaml_config: Optional YAML configuration (triggers CONFIG mode)

    Returns:
        List of fully resolved Context objects. Single-element for the
        common single-arch case; multi-element when YAML declares
        `architecture: [x64, arm64]` (Linux multi-arch).

    Raises:
        ValueError: If required fields missing or invalid

    Modes:
        - CONFIG mode (yaml_config provided): YAML is authoritative
        - DIRECT mode (no yaml_config): CLI > Env > Defaults

    Note:
        root_dir is always computed from package location via get_package_root(),
        never from config or cwd.
    """
    if yaml_config:
        return _resolve_config_mode(yaml_config, cli_args)
    else:
        return _resolve_direct_mode(cli_args)


def _resolve_config_mode(
    yaml_config: Dict[str, Any], cli_args: Dict[str, Any]
) -> List[Context]:
    """CONFIG MODE: YAML is base, CLI can override.

    Args:
        yaml_config: YAML configuration dictionary
        cli_args: CLI arguments (can override YAML values)

    Returns:
        List of Contexts. One per architecture when YAML provides a list.

    Raises:
        ValueError: If required fields missing from both YAML and CLI
    """
    build_section = yaml_config.get("build", {})

    # chromium_src: CLI override > YAML > error
    chromium_src_str = cli_args.get("chromium_src") or build_section.get("chromium_src")
    if not chromium_src_str:
        raise ValueError(
            "CONFIG MODE: chromium_src required in YAML!\n"
            "Add to your config:\n"
            "  build:\n"
            "    chromium_src: /path/to/chromium"
        )

    chromium_src = Path(chromium_src_str)
    chromium_src_source = "cli" if cli_args.get("chromium_src") else "yaml"

    # Validate chromium_src exists
    if not chromium_src.exists():
        raise ValueError(
            f"CONFIG MODE: chromium_src does not exist: {chromium_src}\n"
            f"Expected directory with Chromium source code"
        )

    # architecture: CLI override > YAML > platform default.
    # YAML may be a string OR a list (e.g. [x64, arm64]) — list form runs
    # the entire pipeline once per arch.
    cli_arch = cli_args.get("arch")
    yaml_arch = build_section.get("architecture") or build_section.get("arch")

    if cli_arch:
        architectures = [cli_arch]
        arch_source = "cli"
    elif yaml_arch is not None:
        architectures = yaml_arch if isinstance(yaml_arch, list) else [yaml_arch]
        arch_source = "yaml"
    else:
        architectures = [get_platform_arch()]
        arch_source = "default"
        log_info(
            f"CONFIG MODE: Using platform default architecture: {architectures[0]}"
        )

    for arch in architectures:
        if arch not in VALID_ARCHITECTURES:
            raise ValueError(
                f"CONFIG MODE: invalid architecture '{arch}'. "
                f"Valid: {sorted(VALID_ARCHITECTURES)}"
            )

    # build_type: CLI override > YAML > debug
    build_type = cli_args.get("build_type") or build_section.get("type", "debug")
    build_type_source = "cli" if cli_args.get("build_type") else "yaml"

    log_info(f"✓ CONFIG MODE: chromium_src={chromium_src} ({chromium_src_source})")
    if len(architectures) > 1:
        log_info(
            f"✓ CONFIG MODE: architectures={architectures} ({arch_source}, multi-arch loop)"
        )
    else:
        log_info(
            f"✓ CONFIG MODE: architecture={architectures[0]} ({arch_source})"
        )
    log_info(f"✓ CONFIG MODE: build_type={build_type} ({build_type_source})")

    return [
        Context(
            chromium_src=chromium_src,
            architecture=arch,
            build_type=build_type,
        )
        for arch in architectures
    ]


def _resolve_direct_mode(cli_args: Dict[str, Any]) -> List[Context]:
    """DIRECT MODE: CLI > Env > Defaults.

    Args:
        cli_args: CLI arguments (None if not provided by user)

    Returns:
        Single-element list with the resolved Context. DIRECT mode is
        always single-arch (CLI --arch is a scalar).

    Raises:
        ValueError: If chromium_src not provided
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

    # Validate chromium_src exists
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

    log_info(f"✓ DIRECT MODE: chromium_src={chromium_src} (cli/env)")
    log_info(f"✓ DIRECT MODE: architecture={architecture} (cli/env/default)")
    log_info(f"✓ DIRECT MODE: build_type={build_type} (cli/default)")

    return [
        Context(
            chromium_src=chromium_src,
            architecture=architecture,
            build_type=build_type,
        )
    ]


def resolve_pipeline(
    cli_args: Dict[str, Any],
    yaml_config: Optional[Dict[str, Any]] = None,
    execution_order: Optional[List[Tuple[str, List[str]]]] = None,
) -> List[str]:
    """Resolve build pipeline - single entry point.

    Args:
        cli_args: CLI arguments dictionary
        yaml_config: Optional YAML configuration (triggers CONFIG mode)
        execution_order: Required for DIRECT mode with phase flags

    Returns:
        List of module names in execution order

    Raises:
        ValueError: If no pipeline specified or conflicting modes

    Modes:
        - CONFIG mode: Returns yaml_config["modules"]
        - DIRECT mode: --modules or phase flags
    """
    if yaml_config:
        return _resolve_pipeline_config_mode(yaml_config)
    else:
        return _resolve_pipeline_direct_mode(cli_args, execution_order)


def _resolve_pipeline_config_mode(yaml_config: Dict[str, Any]) -> List[str]:
    """CONFIG MODE: Pipeline from YAML modules list.

    Args:
        yaml_config: YAML configuration dictionary

    Returns:
        Module list from YAML

    Raises:
        ValueError: If modules not specified in YAML
    """
    modules = yaml_config.get("modules")
    if not modules:
        raise ValueError(
            "CONFIG MODE: modules required in YAML!\n"
            "Add to your config:\n"
            "  modules: [clean, configure, compile, sign_macos]"
        )

    log_info(f"✓ CONFIG MODE: pipeline={modules} (yaml)")
    return modules


def _resolve_pipeline_direct_mode(
    cli_args: Dict[str, Any],
    execution_order: Optional[List[Tuple[str, List[str]]]],
) -> List[str]:
    """DIRECT MODE: Pipeline from --modules or phase flags.

    Args:
        cli_args: CLI arguments dictionary
        execution_order: Phase execution order (required for flag mode)

    Returns:
        Module list in execution order

    Raises:
        ValueError: If no pipeline specified or both modes used
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
        log_info(f"✓ DIRECT MODE: pipeline={pipeline} (--modules)")
        return pipeline

    if has_flags:
        if execution_order is None:
            raise ValueError(
                "DIRECT MODE: execution_order required for phase flag resolution"
            )
        pipeline = _build_pipeline_from_flags(cli_args, execution_order)
        log_info(f"✓ DIRECT MODE: pipeline={pipeline} (phase flags)")
        return pipeline

    raise ValueError("DIRECT MODE: Internal error - no pipeline resolution matched")


def _has_phase_flags(cli_args: Dict[str, Any]) -> bool:
    """Check if any phase flags are set.

    Args:
        cli_args: CLI arguments dictionary

    Returns:
        True if any phase flag is True
    """
    phase_flags = ["setup", "prep", "build", "sign", "package", "upload"]
    return any(cli_args.get(flag, False) for flag in phase_flags)


def _build_pipeline_from_flags(
    cli_args: Dict[str, Any],
    execution_order: List[Tuple[str, List[str]]],
) -> List[str]:
    """Build pipeline from phase flags with fixed execution order.

    Args:
        cli_args: CLI arguments with phase flag keys
        execution_order: List of (phase_name, modules) defining order

    Returns:
        Module list in predetermined order
    """
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
