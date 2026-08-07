#!/usr/bin/env python3
"""Exclusive lock for BrowserOS builds that mutate a Chromium checkout."""

import errno
import json
import os
import socket
import sys
from datetime import datetime, timezone
from hashlib import sha256
from pathlib import Path
from typing import IO, Iterable, Optional

if sys.platform == "win32":
    import msvcrt
else:
    import fcntl


class CheckoutLockError(RuntimeError):
    """Raised when another build already owns the Chromium checkout lock."""


class ChromiumCheckoutLock:
    """Process-wide exclusive lock keyed to a resolved Chromium src path.

    BrowserOS release builds run destructive setup (`git reset`, `git clean`,
    patch application) and long compiles in one shared checkout. The lock file
    lives next to the gclient checkout root, not under src/, so `git clean`
    cannot delete it while the build is running.
    """

    def __init__(
        self,
        chromium_src: Path,
        *,
        product: str,
        wait: bool = False,
        command: Optional[Iterable[str]] = None,
    ) -> None:
        self.chromium_src = _resolved(chromium_src)
        self.product = product
        self.wait = wait
        self.command = tuple(command) if command is not None else tuple(sys.argv)
        self.lock_path = lock_path_for(self.chromium_src)
        self._file: Optional[IO[str]] = None

    def __enter__(self) -> "ChromiumCheckoutLock":
        self.lock_path.parent.mkdir(parents=True, exist_ok=True)
        self._file = self.lock_path.open("a+", encoding="utf-8")

        if not _try_lock(self._file, wait=self.wait):
            holder = _read_holder(self._file)
            self._file.close()
            self._file = None
            raise CheckoutLockError(
                _format_contention(
                    chromium_src=self.chromium_src,
                    lock_path=self.lock_path,
                    holder=holder,
                )
            )

        _write_holder(self._file, self._holder_metadata())
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        if self._file is None:
            return
        try:
            self._file.seek(0)
            self._file.truncate()
            self._file.flush()
            os.fsync(self._file.fileno())
        finally:
            _unlock(self._file)
            self._file.close()
            self._file = None

    def _holder_metadata(self) -> dict[str, object]:
        return {
            "pid": os.getpid(),
            "product": self.product,
            "started_at": datetime.now(timezone.utc)
            .isoformat(timespec="seconds")
            .replace("+00:00", "Z"),
            "chromium_src": str(self.chromium_src),
            "lock_path": str(self.lock_path),
            "cwd": os.getcwd(),
            "host": socket.gethostname(),
            "command": list(self.command),
        }


def lock_path_for(chromium_src: Path) -> Path:
    resolved = _resolved(chromium_src)
    digest = sha256(str(resolved).encode("utf-8")).hexdigest()[:12]
    return resolved.parent / ".browseros-build-locks" / f"{resolved.name}-{digest}.lock"


def _resolved(path: Path) -> Path:
    return Path(path).expanduser().resolve(strict=False)


def _try_lock(file: IO[str], *, wait: bool) -> bool:
    if sys.platform == "win32":
        return _try_lock_windows(file, wait=wait)
    flags = fcntl.LOCK_EX
    if not wait:
        flags |= fcntl.LOCK_NB
    try:
        fcntl.flock(file.fileno(), flags)
        return True
    except OSError as e:
        if not wait and e.errno in (errno.EACCES, errno.EAGAIN):
            return False
        raise


def _unlock(file: IO[str]) -> None:
    if sys.platform == "win32":
        file.seek(0)
        msvcrt.locking(file.fileno(), msvcrt.LK_UNLCK, 1)
        return
    fcntl.flock(file.fileno(), fcntl.LOCK_UN)


def _try_lock_windows(file: IO[str], *, wait: bool) -> bool:
    # Windows locks byte ranges. Ensure there is at least one byte to lock.
    file.seek(0, os.SEEK_END)
    if file.tell() == 0:
        file.write("\0")
        file.flush()
    file.seek(0)
    mode = msvcrt.LK_LOCK if wait else msvcrt.LK_NBLCK
    try:
        msvcrt.locking(file.fileno(), mode, 1)
        return True
    except OSError as e:
        if not wait and e.errno in (errno.EACCES, errno.EDEADLK):
            return False
        raise


def _write_holder(file: IO[str], metadata: dict[str, object]) -> None:
    file.seek(0)
    file.truncate()
    json.dump(metadata, file, indent=2, sort_keys=True)
    file.write("\n")
    file.flush()
    os.fsync(file.fileno())


def _read_holder(file: IO[str]) -> Optional[dict[str, object]]:
    try:
        file.seek(0)
        content = file.read().strip("\0 \n\t")
        if not content:
            return None
        data = json.loads(content)
        return data if isinstance(data, dict) else None
    except (OSError, json.JSONDecodeError):
        return None


def _format_contention(
    *, chromium_src: Path, lock_path: Path, holder: Optional[dict[str, object]]
) -> str:
    lines = [
        f"Chromium checkout is already locked: {chromium_src}",
        "Another BrowserOS build is using this checkout; refusing to run because "
        "setup, patch, compile, sign, and package steps mutate shared state.",
    ]
    if holder:
        lines.append(
            "Holder: "
            f"pid={holder.get('pid', 'unknown')} "
            f"product={holder.get('product', 'unknown')} "
            f"started_at={holder.get('started_at', 'unknown')} "
            f"host={holder.get('host', 'unknown')}"
        )
        cwd = holder.get("cwd")
        if cwd:
            lines.append(f"Holder cwd: {cwd}")
        command = holder.get("command")
        if isinstance(command, list) and command:
            lines.append(f"Holder command: {' '.join(str(part) for part in command)}")
    else:
        lines.append("Holder: unknown")
    lines.append(f"Lock file: {lock_path}")
    lines.append("Use --lock-wait to wait for the current build to finish.")
    return "\n".join(lines)
