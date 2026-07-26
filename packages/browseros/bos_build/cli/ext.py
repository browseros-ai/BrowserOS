#!/usr/bin/env python3
"""Extension packaging & release commands.

Kept in its own module (browseros.py just add_typers it) mirroring
cli/release_feeds.py; context/execute helpers stay local for the same
circular-import reason documented there.
"""

from typing import List, Optional

import typer

from ..core.context import Context
from ..core.runner import StepExecutionError, run as run_steps
from ..lib.notify import slack_subscriber
from ..lib.paths import get_package_root
from ..lib.utils import log_error
from ..release.extensions.release import build_pipeline
from ..release.extensions.specs import EXTENSION_SPECS

app = typer.Typer(
    help="Extension packaging & release",
    pretty_exceptions_enable=False,
    pretty_exceptions_show_locals=False,
)

_NAMES = ", ".join(spec.name for spec in EXTENSION_SPECS)


def _create_context(version: str) -> Context:
    root = get_package_root()
    try:
        ctx = Context(
            root_dir=root,
            chromium_src=root,
            architecture="",
            build_type="release",
            product="browseros",
        )
    except ValueError as e:
        log_error(str(e))
        raise typer.Exit(1)
    ctx.release_version = version
    return ctx


def _execute(ctx: Context, steps: List) -> None:
    try:
        run_steps(ctx, steps, name="ext-release", subscribers=(slack_subscriber(ctx),))
    except StepExecutionError as e:
        log_error(str(e))
        raise typer.Exit(1)
    except KeyboardInterrupt:
        raise typer.Exit(130)


@app.command("release")
def release(
    version: str = typer.Option(
        ..., "--version", "-v", help="Version stamped on every selected extension"
    ),
    name: Optional[str] = typer.Option(
        None, "--name", "-n", help=f"One extension ({_NAMES}); default: all"
    ),
    branch: Optional[str] = typer.Option(
        None,
        "--branch",
        help="Branch override for external-repo extensions "
        "(in-repo extensions build the current working tree)",
    ),
    chrome_binary: Optional[str] = typer.Option(
        None, "--chrome-binary", help="Chrome binary for --pack-extension"
    ),
):
    """Build, pack, and upload extension CRXs.

    In-repo extensions (agent, browserclaw) build from this working tree and
    stamp --version into their package.json.

    \b
    Release the agent CRX:
      browseros ext release --version 0.0.118 --name agent

    \b
    Update feeds separately (dry run unless --publish is passed):
      browseros release extensions --channel alpha --set agent=0.0.118
    """
    try:
        steps = build_pipeline(
            version=version,
            name=name,
            branch=branch,
            chrome_binary=chrome_binary,
        )
    except ValueError as e:
        log_error(str(e))
        raise typer.Exit(1)

    _execute(_create_context(version), steps)
