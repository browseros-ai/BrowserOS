"""Prepare the infrastructure-owned macOS cache and clone isolated product inputs.

The base checkout lock spans source refresh and every APFS copy. Ownership
markers and product-qualified state files also cover unfinished copies, so a
failed job can remove its files without touching another product or checkout.
"""

import re
import shutil
import subprocess
import sys
import tempfile
from contextlib import ExitStack
from pathlib import Path
from typing import Sequence

from ...core.checkout_lock import CheckoutLockError, ChromiumCheckoutLock
from ...lib.utils import log_info, log_warning
from ..setup.clean import clean_ci_outputs
from .provision import append_github_file, ensure

PRODUCTS = ("browseros", "browserclaw")
WORKSPACE_PARENT = "browseros-ci-apfs-workspaces"
WORKSPACE_PREFIX = "browseros-ci-chromium-"
MARKER = ".browseros-workspace-state.env"
LOCKS = ".browseros-build-locks"


def _validate_request(products: Sequence[str], run_tag: str) -> None:
    if not products or len(set(products)) != len(products):
        raise ValueError("Expected a nonempty, unique product list")
    if any(product not in PRODUCTS for product in products):
        raise ValueError(f"Unknown product; expected one of {PRODUCTS}")
    if not re.fullmatch(r"[A-Za-z0-9_]+-[A-Za-z0-9_]+", run_tag):
        raise ValueError("Workspace run tag must be <run-id>-<attempt>")


def _state_path(runner_temp: Path, run_tag: str, product: str) -> Path:
    return runner_temp / f"browseros-ci-chromium-workspace-{run_tag}-{product}.env"


def _write_state(path: Path, state: dict[str, str]) -> None:
    if path.is_symlink():
        raise ValueError(f"Refusing symlink workspace state: {path}")
    if any("\n" in value or "\r" in value for value in state.values()):
        raise ValueError("Workspace paths and metadata must fit on one line")
    # The runner state is a recovery ledger, not a ready signal. Replace it
    # atomically before copying; a killed cp must still leave an owned target.
    with tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", dir=path.parent, delete=False
    ) as pending:
        pending.write("".join(f"{key}={value}\n" for key, value in state.items()))
        pending_path = Path(pending.name)
    try:
        pending_path.replace(path)
    finally:
        pending_path.unlink(missing_ok=True)


def _read_state(path: Path, *, allow_legacy: bool = False) -> dict[str, str]:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"Missing regular workspace state file: {path}")
    state = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        key, separator, value = line.partition("=")
        if not separator or key in state:
            raise ValueError(f"Malformed workspace state: {path}")
        state[key] = value
    # Previous releases used one unqualified workspace. Accept that exact old
    # shape only for marker-based reaping; new ledgers always identify products.
    legacy = allow_legacy and "product" not in state
    if legacy:
        state["product"] = "browseros"
    _validate_request([state.get("product", "")], state.get("run_tag", ""))
    base_root = Path(state.get("base_root", ""))
    parent = base_root.parent / WORKSPACE_PARENT
    suffix = state["run_tag"] if legacy else f"{state['run_tag']}-{state['product']}"
    root = parent / f"{WORKSPACE_PREFIX}{suffix}"
    expected = {
        "base_root": str(base_root),
        "base_src": str(base_root / "src"),
        "workspace_parent": str(parent),
        "workspace_root": str(root),
        "workspace_src": str(root / "src"),
    }
    if not base_root.is_absolute() or any(
        state.get(key) != value or Path(value).resolve() != Path(value)
        for key, value in expected.items()
    ):
        raise ValueError(f"Unexpected workspace paths in {path}")
    if base_root == parent or base_root == root or base_root.parent == parent:
        raise ValueError(f"Workspace cannot be its own source cache: {path}")
    if (base_root / LOCKS).is_symlink() or (root / LOCKS).is_symlink():
        raise ValueError(f"Refusing symlink checkout lock directory in {path}")
    return state


def _same_owner(first: dict[str, str], second: dict[str, str]) -> bool:
    return all(
        first.get(key) == second.get(key)
        for key in ("workspace_root", "base_root", "run_tag", "product")
    )


def _remove_contents(state: dict[str, str]) -> None:
    """Remove owned files while the caller holds base and destination locks."""
    root = Path(state["workspace_root"])
    marker_state = _read_state(root / MARKER, allow_legacy=True)
    if not _same_owner(state, marker_state):
        raise ValueError(f"Workspace ownership marker does not match: {root}")
    # ChromiumCheckoutLock lives inside the gclient root. Deleting that inode,
    # even after unlocking, lets a waiting process and a new opener lock two
    # different files for the same src path. Keep a small owned tombstone.
    for entry in root.iterdir():
        if entry.name in (LOCKS, MARKER):
            continue
        if entry.is_symlink() or not entry.is_dir():
            entry.unlink()
        else:
            shutil.rmtree(entry)


def _reap_stale_workspaces(parent: Path, base_root: Path, run_tag: str) -> None:
    # Same-run products are peers, even when a later invocation requests only
    # one of them. Never reap a peer as a side effect of setting up its sibling.
    for root in sorted(parent.glob(f"{WORKSPACE_PREFIX}*")):
        if root.is_symlink() or not root.is_dir():
            continue
        try:
            state = _read_state(root / MARKER, allow_legacy=True)
            if (
                state["workspace_root"] != str(root)
                or state["base_root"] != str(base_root)
                or state["run_tag"] == run_tag
            ):
                continue
            with ChromiumCheckoutLock(root / "src", product=state["product"]):
                _remove_contents(state)
        except (OSError, ValueError, CheckoutLockError) as error:
            log_warning(f"[workspace] Leaving stale candidate {root}: {error}")


def _verify_clone_support(parent: Path) -> None:
    with tempfile.TemporaryDirectory(
        prefix=".browseros-apfs-probe-", dir=parent
    ) as tmp:
        source = Path(tmp) / "source"
        target = Path(tmp) / "clone"
        source.write_bytes(b"browseros apfs clone probe\n")
        subprocess.run(["cp", "-c", str(source), str(target)], check=True)
        if target.read_bytes() != source.read_bytes():
            raise RuntimeError("APFS clone probe produced different bytes")


def _copy_base(base_root: Path, workspace_root: Path) -> None:
    # Copy children explicitly: copying root/. would overwrite the destination
    # lock directory with base lock files and break checkout ownership.
    for entry in sorted(base_root.iterdir()):
        if entry.name in (LOCKS, MARKER):
            continue
        subprocess.run(
            ["cp", "-cR", str(entry), str(workspace_root / entry.name)], check=True
        )


def _head(src: Path) -> str:
    return subprocess.check_output(
        ["git", "-C", str(src), "rev-parse", "HEAD"], text=True
    ).strip()


def prepare_workspaces(
    base_src: Path,
    version: str,
    products: Sequence[str],
    runner_temp: Path,
    run_tag: str,
) -> dict[str, Path]:
    """Refresh one CI base and return every requested product's prepared src.

    This is destructive only within the explicit infrastructure-owned base and
    marker-owned destinations. No ready output is published until all copies
    succeed; the per-product recovery ledger exists before each copy starts.
    """
    _validate_request(products, run_tag)
    if sys.platform != "darwin":
        raise ValueError("Disposable APFS Chromium workspaces require macOS")
    base_src = base_src.expanduser().resolve(strict=True)
    runner_temp = runner_temp.expanduser().resolve(strict=True)
    base_root = base_src.parent
    if (
        base_src.name != "src"
        or not (base_src / ".git").is_dir()
        or not (base_root / ".gclient").is_file()
        or base_root.name == WORKSPACE_PARENT
        or base_root.parent.name == WORKSPACE_PARENT
    ):
        raise ValueError(
            f"Expected a standalone persistent gclient checkout: {base_src}"
        )
    parent = base_root.parent / WORKSPACE_PARENT
    if parent.is_symlink():
        raise ValueError(f"Workspace parent must not be a symlink: {parent}")
    parent.mkdir(exist_ok=True)
    if parent.stat().st_dev != base_root.stat().st_dev:
        raise ValueError("Chromium base and workspace parent must share an APFS volume")
    _verify_clone_support(parent)

    states: list[dict[str, str]] = []
    requested = []
    for product in products:
        root = parent / f"{WORKSPACE_PREFIX}{run_tag}-{product}"
        state = {
            "workspace_parent": str(parent),
            "workspace_root": str(root),
            "workspace_src": str(root / "src"),
            "base_root": str(base_root),
            "base_src": str(base_src),
            "base_head": "",
            "chromium_version": version,
            "run_tag": run_tag,
            "product": product,
        }
        # Validate roles before source mutation, then check markers again under
        # locks. Existing unmarked paths are never adopted as CI workspaces.
        if root.exists() or root.is_symlink():
            if not _same_owner(state, _read_state(root / MARKER)):
                raise ValueError(f"Refusing unowned workspace: {root}")
        if (root / LOCKS).is_symlink():
            raise ValueError(
                f"Refusing symlink checkout lock directory: {root / LOCKS}"
            )
        requested.append(state)
    if (base_root / LOCKS).is_symlink():
        raise ValueError(
            f"Refusing symlink checkout lock directory: {base_root / LOCKS}"
        )

    with (
        ChromiumCheckoutLock(base_src, product="source-workspace"),
        ExitStack() as locks,
    ):
        _reap_stale_workspaces(parent, base_root, run_tag)
        try:
            for state in requested:
                root = Path(state["workspace_root"])
                product = state["product"]
                if not root.exists():
                    root.mkdir()
                    _write_state(root / MARKER, state)
                # Always acquire base before destination. Hold every copy lock
                # until publication, so the batch has one ready handoff.
                locks.enter_context(ChromiumCheckoutLock(root / "src", product=product))
                _remove_contents(state)
                states.append(state)
                _write_state(root / MARKER, state)
                _write_state(_state_path(runner_temp, run_tag, product), state)

            # Acquire destinations before refreshing the base, so an active
            # build aborts a retry without mutating source behind that build.
            # Cleanup must precede ensure's final sync and restoring hooks.
            clean_ci_outputs(base_src)
            prepared_src = ensure(base_root, version, strategy="shallow", reset=True)
            base_head = _head(prepared_src)
            for state in states:
                root = Path(state["workspace_root"])
                product = state["product"]
                state["base_head"] = base_head
                _write_state(root / MARKER, state)
                _write_state(_state_path(runner_temp, run_tag, product), state)
                log_info(f"[workspace] Cloning prepared source for {product}: {root}")
                _copy_base(base_root, root)
                if _head(root / "src") != base_head:
                    raise RuntimeError(f"APFS clone HEAD does not match base: {root}")
        except BaseException:
            for state in states:
                try:
                    _remove_contents(state)
                    _state_path(runner_temp, run_tag, state["product"]).unlink(
                        missing_ok=True
                    )
                except Exception as error:
                    log_warning(f"[workspace] Recovery ledger retained: {error}")
            raise

        # Environment/step outputs are ready signals, unlike the recovery
        # ledgers above. A partial second copy must publish neither product.
        for state in states:
            product = state["product"]
            for key in ("workspace_src", "workspace_root"):
                output_key = "chromium_src" if key == "workspace_src" else key
                append_github_file(
                    "GITHUB_OUTPUT", f"{product}_{output_key}={state[key]}"
                )
            path = _state_path(runner_temp, run_tag, product)
            append_github_file("GITHUB_OUTPUT", f"{product}_state_path={path}")
        append_github_file("GITHUB_OUTPUT", f"base_src={base_src}")
        append_github_file("GITHUB_OUTPUT", f"base_head={base_head}")
        if len(states) == 1:
            state = states[0]
            path = _state_path(runner_temp, run_tag, state["product"])
            append_github_file("GITHUB_ENV", f"CHROMIUM_SRC={state['workspace_src']}")
            append_github_file(
                "GITHUB_OUTPUT", f"chromium_src={state['workspace_src']}"
            )
            append_github_file(
                "GITHUB_OUTPUT", f"workspace_root={state['workspace_root']}"
            )
            append_github_file("GITHUB_OUTPUT", f"state_path={path}")
    return {state["product"]: Path(state["workspace_src"]) for state in states}


def cleanup_workspaces(
    runner_temp: Path,
    run_tag: str,
    products: Sequence[str] = PRODUCTS,
) -> None:
    """Clean this run's recovery ledgers, retaining markers for refused targets."""
    _validate_request(products, run_tag)
    runner_temp = runner_temp.expanduser().resolve(strict=True)
    failures = []
    for product in products:
        path = _state_path(runner_temp, run_tag, product)
        if not path.exists() and not path.is_symlink():
            continue
        try:
            state = _read_state(path)
            if state["run_tag"] != run_tag or state["product"] != product:
                raise ValueError(f"Recovery ledger ownership does not match: {path}")
            root = Path(state["workspace_root"])
            with (
                ChromiumCheckoutLock(
                    Path(state["base_src"]), product="source-workspace"
                ),
                ChromiumCheckoutLock(root / "src", product=product),
            ):
                _remove_contents(state)
                path.unlink()
        except (OSError, ValueError, CheckoutLockError) as error:
            failures.append(f"{product}: {error}")
    if failures:
        raise RuntimeError(
            "Workspace cleanup refused; recovery ledgers retained:\n"
            + "\n".join(failures)
        )
