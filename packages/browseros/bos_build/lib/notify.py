#!/usr/bin/env python3
"""Slack notifications for BrowserOS build pipeline lifecycle events."""

import os
import threading
from dataclasses import dataclass
from typing import Any, Dict, Optional, Sequence

from .utils import IS_LINUX, IS_MACOS, IS_WINDOWS

# Slack attachment colors
COLOR_BLUE = "#2196F3"
COLOR_GREEN = "#4CAF50"
COLOR_RED = "#F44336"

_OS_EMOJI = {
    "macOS": "🍎",
    "Windows": "🪟",
    "Linux": "🐧",
}


class Notifier:
    """Fire-and-forget notification system"""

    def __init__(self):
        self.slack_webhook_url = os.environ.get("SLACK_WEBHOOK_URL")
        self.enabled = bool(self.slack_webhook_url)

    def notify(
        self,
        event: str,
        message: str,
        details: Optional[Dict[str, Any]] = None,
        color: str = "#36a64f",
        wait: bool = False,
        footer: str = "BrowserOS Build System",
    ) -> None:
        """Send notification; fire-and-forget unless wait=True.

        Terminal (end-of-run) notifications must pass wait=True: daemon
        threads die with the process, and the final send historically
        raced process exit — the cause of runs that never notified at
        the end.
        """
        if not self.enabled:
            return

        if wait:
            self._send_notification(event, message, details, color, footer)
            return

        thread = threading.Thread(
            target=self._send_notification,
            args=(event, message, details, color, footer),
            daemon=True,
        )
        thread.start()

    def _send_notification(
        self,
        event: str,
        message: str,
        details: Optional[Dict[str, Any]],
        color: str,
        footer: str,
    ) -> None:
        """Internal method to send notification (runs in background thread)"""
        try:
            import requests

            # Use legacy attachment format for colored sidebar
            attachment = {
                "color": color,
                "mrkdwn_in": ["text", "fields"],
                "text": f"*{event}*\n{message}",
                "footer": footer,
            }

            if details:
                attachment["fields"] = [
                    {"title": key, "value": str(value), "short": True}
                    for key, value in details.items()
                ]

            payload = {"attachments": [attachment]}

            requests.post(
                self.slack_webhook_url,
                json=payload,
                timeout=5,  # Quick timeout for fire-and-forget
            )

        except ImportError:
            pass
        except Exception:
            pass


def format_duration(seconds: float) -> str:
    """Format seconds for Slack messages without leaking raw float durations."""
    total_seconds = max(0, int(round(seconds)))
    if total_seconds < 60:
        return f"{total_seconds}s"

    minutes, remaining_seconds = divmod(total_seconds, 60)
    if remaining_seconds == 0:
        return f"{minutes}m"
    return f"{minutes}m {remaining_seconds}s"


@dataclass
class _FailedStep:
    step: str
    phase: str
    error: str


class SlackRunSubscriber:
    """Stateful event-bus subscriber for one build/release run."""

    def __init__(self, ctx, notifier: Optional[Notifier] = None):
        self.notifier = notifier or Notifier()
        self.display_name = ctx.product.display_name
        self.version = ctx.release_version or ctx.semantic_version
        self.os_name = _current_os_name()
        self.architecture = ctx.architecture
        self.identity = self._identity()
        self.footer = self._footer()
        self.artifact_registry = getattr(ctx, "artifact_registry", None)

        self.current_phase = ""
        self.current_step = ""
        self.phase_durations: Dict[str, float] = {}
        self.last_failed_step: Optional[_FailedStep] = None

    def __call__(self, event) -> None:
        """Consume a runner event; runner._emit logs subscriber failures."""
        self._handle(event)

    def _handle(self, event) -> None:
        from ..core.events import RunFinished, RunStarted, StepFinished, StepStarted

        if isinstance(event, RunStarted):
            self._run_started(event)
        elif isinstance(event, StepStarted):
            self._step_started(event)
        elif isinstance(event, StepFinished):
            self._step_finished(event)
        elif isinstance(event, RunFinished):
            self._run_finished(event)

    def _run_started(self, event) -> None:
        title = f"🚀 {_run_label(event.run)} started — {self.identity}"
        chain = _planned_phase_chain(event.steps)
        self._send(title, chain, color=COLOR_BLUE)

    def _step_started(self, event) -> None:
        self.current_step = event.step
        if not event.phase:
            return
        if not self.current_phase:
            self.current_phase = event.phase
            return
        if event.phase == self.current_phase:
            return

        previous_phase = self.current_phase
        duration = self.phase_durations.get(previous_phase, 0.0)
        title = (
            f"✅ {_phase_label(previous_phase)} done ({format_duration(duration)}) "
            f"→ {_phase_label(event.phase)}"
        )
        self._send(title, "", color=COLOR_GREEN)
        self.current_phase = event.phase

    def _step_finished(self, event) -> None:
        if event.phase:
            self.phase_durations[event.phase] = (
                self.phase_durations.get(event.phase, 0.0) + event.duration
            )
        if event.status == "failed":
            self.last_failed_step = _FailedStep(
                step=event.step,
                phase=event.phase,
                error=event.error or "",
            )
        elif event.status == "success":
            self.current_step = ""

    def _run_finished(self, event) -> None:
        if event.status == "success":
            self._run_succeeded(event)
        elif event.status == "interrupted":
            self._run_interrupted(event)
        else:
            self._run_failed(event)

    def _run_succeeded(self, event) -> None:
        details = self._artifact_fields()
        self._send(
            f"🏁 {_run_label(event.run)} completed — {self.identity}",
            f"Completed in {format_duration(event.duration)}",
            details=details,
            color=COLOR_GREEN,
            wait=True,
        )

    def _run_failed(self, event) -> None:
        failed = self.last_failed_step
        if failed and failed.step:
            phase = f" ({_phase_label(failed.phase)} phase)" if failed.phase else ""
            title = (
                f"❌ {_run_label(event.run)} FAILED at '{failed.step}'{phase} "
                f"— {self.identity}"
            )
            error = failed.error or event.error or event.status
        else:
            title = f"❌ {_run_label(event.run)} FAILED — {self.identity}"
            error = event.error or event.status

        self._send(
            title,
            f"Terminated after {format_duration(event.duration)}",
            details={"Error": error},
            color=COLOR_RED,
            wait=True,
        )

    def _run_interrupted(self, event) -> None:
        if self.current_step:
            title = (
                f"🛑 {_run_label(event.run)} interrupted at '{self.current_step}' "
                f"— {self.identity}"
            )
        else:
            title = f"🛑 {_run_label(event.run)} interrupted — {self.identity}"
        self._send(
            title,
            f"after {format_duration(event.duration)}",
            color=COLOR_RED,
            wait=True,
        )

    def _artifact_fields(self) -> Optional[Dict[str, str]]:
        if self.artifact_registry is None:
            return None
        release_links = self.artifact_registry.get("release_links")
        if not release_links:
            return None
        try:
            entries = iter(release_links)
        except TypeError:
            return None

        links = []
        for entry in entries:
            if not isinstance(entry, (list, tuple)) or len(entry) != 2:
                continue
            filename, url = entry
            if not filename or not url:
                continue
            links.append(f"<{url}|{filename}>")
        if not links:
            return None
        return {"Artifacts": "\n".join(links)}

    def _send(
        self,
        title: str,
        body: str,
        *,
        details: Optional[Dict[str, Any]] = None,
        color: str,
        wait: bool = False,
    ) -> None:
        self.notifier.notify(
            title,
            body,
            details,
            color=color,
            wait=wait,
            footer=self.footer,
        )

    def _identity(self) -> str:
        parts = [f"{self.display_name} v{self.version}", f"· {self.os_name}"]
        if self.architecture:
            parts.append(self.architecture)
        return " ".join(parts)

    def _footer(self) -> str:
        emoji = _OS_EMOJI.get(self.os_name, "")
        prefix = f"{emoji} " if emoji else ""
        return f"{prefix}{self.display_name} Build · {self.os_name}"


def slack_subscriber(ctx) -> SlackRunSubscriber:
    """Create a per-run Slack event subscriber bound to the run context."""
    return SlackRunSubscriber(ctx)


def _current_os_name() -> str:
    if IS_MACOS():
        return "macOS"
    if IS_WINDOWS():
        return "Windows"
    if IS_LINUX():
        return "Linux"
    return "Unknown"


def _planned_phase_chain(step_names: Sequence[str]) -> str:
    from ..core.step import all_steps

    registry = all_steps()
    phases = []
    seen = set()
    for step_name in step_names:
        step_cls = registry.get(step_name)
        phase = step_cls.phase if step_cls else ""
        if phase and phase not in seen:
            phases.append(phase)
            seen.add(phase)
    return " → ".join(phases)


def _phase_label(phase: str) -> str:
    return phase.replace("_", " ").title()


def _run_label(run: str) -> str:
    if run.lower() == "ota":
        return "OTA"
    return " ".join(part.capitalize() for part in run.replace("_", "-").split("-"))
