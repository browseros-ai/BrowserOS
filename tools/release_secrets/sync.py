#!/usr/bin/env python3
"""Sync allowlisted release workflow secrets from a local dotenv file.

Values are never printed. Apply mode sends each value to `gh secret set` over
stdin so secret material does not enter argv, shell history, or temp files.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, Sequence


DEFAULT_REPO = "browseros-ai/BrowserOS"
REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_ENV_FILE = REPO_ROOT / ".env.production"

RELEASE_WORKFLOW_FILES = (
    Path(".github/workflows/build-browseros.yml"),
    Path(".github/workflows/release-browseros.yml"),
    Path(".github/workflows/release-browserclaw.yml"),
    Path(".github/workflows/release-windows.yml"),
    Path(".github/workflows/release-extension-feeds.yml"),
    Path(".github/workflows/release-extensions.yml"),
    Path(".github/workflows/release-server.yml"),
    Path(".github/workflows/release-claw-server.yml"),
)

KEY_RE = re.compile(r"[ \t]*(?:export[ \t]+)?([A-Za-z_][A-Za-z0-9_]*)[ \t]*=")
SECRET_REF_RE = re.compile(
    r"secrets\.([A-Za-z_][A-Za-z0-9_]*)"
    r"|secrets\[['\"]([A-Za-z_][A-Za-z0-9_]*)['\"]\]"
)


class DotenvParseError(ValueError):
    """Raised when the dotenv file cannot be parsed safely."""


@dataclass(frozen=True)
class SecretSpec:
    name: str
    consumers: tuple[str, ...]


@dataclass(frozen=True)
class PlannedSecret:
    name: str
    status: str
    consumers: tuple[str, ...]


@dataclass(frozen=True)
class CheckResult:
    present: list[str]
    automatic: list[str]
    external: list[str]
    optional: list[str]
    missing_required: list[str]


ALLOWLIST: tuple[SecretSpec, ...] = (
    SecretSpec(
        "R2_ACCOUNT_ID",
        (
            "build-browseros.yml",
            "release-browseros.yml",
            "release-browserclaw.yml",
            "release-server.yml",
            "release-claw-server.yml",
            "release-extension-feeds.yml",
            "release-extensions.yml",
        ),
    ),  # Release artifact downloads/uploads.
    SecretSpec(
        "R2_ACCESS_KEY_ID",
        (
            "build-browseros.yml",
            "release-browseros.yml",
            "release-browserclaw.yml",
            "release-server.yml",
            "release-claw-server.yml",
            "release-extension-feeds.yml",
            "release-extensions.yml",
        ),
    ),  # Release artifact downloads/uploads.
    SecretSpec(
        "R2_SECRET_ACCESS_KEY",
        (
            "build-browseros.yml",
            "release-browseros.yml",
            "release-browserclaw.yml",
            "release-server.yml",
            "release-claw-server.yml",
            "release-extension-feeds.yml",
            "release-extensions.yml",
        ),
    ),  # Release artifact downloads/uploads.
    SecretSpec(
        "R2_BUCKET",
        (
            "build-browseros.yml",
            "release-browseros.yml",
            "release-browserclaw.yml",
            "release-server.yml",
            "release-claw-server.yml",
            "release-extension-feeds.yml",
            "release-extensions.yml",
        ),
    ),  # Release artifact downloads/uploads.
    SecretSpec(
        "BROWSEROS_CONFIG_URL",
        ("release-browseros.yml", "release-server.yml"),
    ),  # BrowserOS server inline production config URL.
    SecretSpec(
        "POSTHOG_API_KEY",
        ("release-browseros.yml", "release-server.yml", "release-extensions.yml"),
    ),  # Server and extension release analytics key.
    SecretSpec(
        "SENTRY_DSN",
        ("release-browseros.yml", "release-server.yml"),
    ),  # BrowserOS server inline Sentry DSN.
    SecretSpec(
        "AGENT_RUNNER_JWT_SECRET",
        ("release-browseros.yml", "release-server.yml"),
    ),  # Optional BrowserOS server remote-agent JWT signing secret.
    SecretSpec(
        "ESIGNER_USERNAME",
        (
            "build-browseros.yml",
            "release-browseros.yml",
            "release-browserclaw.yml",
            "release-windows.yml",
        ),
    ),  # Windows signing preflight and CodeSignTool auth.
    SecretSpec(
        "ESIGNER_PASSWORD",
        (
            "build-browseros.yml",
            "release-browseros.yml",
            "release-browserclaw.yml",
            "release-windows.yml",
        ),
    ),  # Windows signing preflight and CodeSignTool auth.
    SecretSpec(
        "ESIGNER_TOTP_SECRET",
        (
            "build-browseros.yml",
            "release-browseros.yml",
            "release-browserclaw.yml",
            "release-windows.yml",
        ),
    ),  # Windows signing preflight and CodeSignTool auth.
    SecretSpec(
        "ESIGNER_CREDENTIAL_ID",
        ("build-browseros.yml",),
    ),  # Optional SSL.com credential selector used by the builder.
    SecretSpec(
        "SPARKLE_PRIVATE_KEY",
        (
            "build-browseros.yml",
            "release-browseros.yml",
            "release-browserclaw.yml",
            "release-windows.yml",
            "release-server.yml",
            "release-claw-server.yml",
        ),
    ),  # Sparkle/WinSparkle artifact signatures and optional server OTA.
    SecretSpec(
        "MACOS_CERTIFICATE_NAME",
        ("build-browseros.yml",),
    ),  # macOS signing certificate identity.
    SecretSpec(
        "PROD_MACOS_NOTARIZATION_APPLE_ID",
        ("build-browseros.yml",),
    ),  # macOS notarization account.
    SecretSpec(
        "PROD_MACOS_NOTARIZATION_TEAM_ID",
        ("build-browseros.yml",),
    ),  # macOS notarization team.
    SecretSpec(
        "PROD_MACOS_NOTARIZATION_PWD",
        ("build-browseros.yml",),
    ),  # macOS notarization app-specific password.
    SecretSpec(
        "BROWSEROS_AGENT_V2_KEY",
        ("release-browseros.yml", "release-extensions.yml"),
    ),  # BrowserOS agent extension signing key.
    SecretSpec(
        "BROWSEROS_CONTROLLER_KEY",
        ("release-extensions.yml",),
    ),  # BrowserOS controller extension signing key.
    SecretSpec(
        "BUGREPORTER_KEY",
        ("release-extensions.yml",),
    ),  # Bug Reporter extension signing key.
    SecretSpec(
        "BROWSERCLAW_KEY",
        ("release-browserclaw.yml", "release-extensions.yml"),
    ),  # BrowserClaw extension signing key.
    SecretSpec(
        "VITE_PUBLIC_SENTRY_DSN",
        ("release-extensions.yml",),
    ),  # Extension build-time Sentry DSN.
    SecretSpec(
        "SENTRY_AUTH_TOKEN",
        ("release-extensions.yml",),
    ),  # Extension sourcemap upload auth.
    SecretSpec(
        "SENTRY_ORG",
        ("release-extensions.yml",),
    ),  # Extension sourcemap upload org.
    SecretSpec(
        "SENTRY_PROJECT",
        ("release-extensions.yml",),
    ),  # Extension sourcemap upload project.
    SecretSpec(
        "VITE_PUBLIC_POSTHOG_KEY",
        ("release-extensions.yml",),
    ),  # Extension build-time PostHog key.
    SecretSpec(
        "VITE_PUBLIC_POSTHOG_HOST",
        ("release-extensions.yml",),
    ),  # Extension build-time PostHog host.
)

KNOWN_AUTOMATIC_SECRETS = frozenset({"GITHUB_TOKEN"})
KNOWN_EXTERNAL_SECRETS = frozenset(
    {
        "GH_TOKEN",
        "MACOS_CERTIFICATE_P12",
        "MACOS_CERTIFICATE_PWD",
        "MACOS_KEYCHAIN_PASSWORD",
    }
)
KNOWN_OPTIONAL_SECRETS = frozenset(
    {"AGENT_RUNNER_JWT_SECRET", "ESIGNER_CREDENTIAL_ID"}
)


def parse_dotenv_file(path: Path) -> dict[str, str]:
    return parse_dotenv_text(path.read_text(encoding="utf-8"))


def parse_dotenv_text(text: str) -> dict[str, str]:
    """Parse dotenv text with quoted multi-line value support."""
    text = text.lstrip("\ufeff").replace("\r\n", "\n").replace("\r", "\n")
    entries: dict[str, str] = {}
    pos = 0
    line_no = 1

    while pos < len(text):
        pos, line_no = _skip_blank_and_comment_lines(text, pos, line_no)
        if pos >= len(text):
            break

        match = KEY_RE.match(text, pos)
        if not match:
            raise DotenvParseError(f"Invalid dotenv syntax at line {line_no}")

        key = match.group(1)
        pos = match.end()
        while pos < len(text) and text[pos] in " \t":
            pos += 1

        if pos < len(text) and text[pos] in ("'", '"'):
            value, pos, line_no = _parse_quoted_value(text, pos, line_no)
            pos, line_no = _consume_trailing_comment(text, pos, line_no)
        else:
            value, pos, line_no = _parse_unquoted_value(text, pos, line_no)

        entries[key] = value

    return entries


def _skip_blank_and_comment_lines(
    text: str, pos: int, line_no: int
) -> tuple[int, int]:
    while pos < len(text):
        cursor = pos
        while cursor < len(text) and text[cursor] in " \t":
            cursor += 1
        if cursor >= len(text):
            return cursor, line_no
        if text[cursor] == "\n":
            pos = cursor + 1
            line_no += 1
            continue
        if text[cursor] == "#":
            newline = text.find("\n", cursor)
            if newline == -1:
                return len(text), line_no
            pos = newline + 1
            line_no += 1
            continue
        return pos, line_no
    return pos, line_no


def _parse_quoted_value(
    text: str, pos: int, line_no: int
) -> tuple[str, int, int]:
    quote = text[pos]
    start_line = line_no
    pos += 1
    chars: list[str] = []

    while pos < len(text):
        char = text[pos]

        if quote == '"' and char == "\\":
            if pos + 1 >= len(text):
                chars.append("\\")
                pos += 1
                continue
            escaped = text[pos + 1]
            if escaped == "\n":
                line_no += 1
            chars.append(_decode_double_quoted_escape(escaped))
            pos += 2
            continue

        if quote == "'" and char == "\\" and pos + 1 < len(text):
            escaped = text[pos + 1]
            if escaped in ("'", "\\"):
                chars.append(escaped)
                pos += 2
                continue

        if char == quote:
            return "".join(chars), pos + 1, line_no

        if char == "\n":
            line_no += 1
        chars.append(char)
        pos += 1

    raise DotenvParseError(f"Unterminated quoted value starting at line {start_line}")


def _decode_double_quoted_escape(char: str) -> str:
    replacements = {
        "n": "\n",
        "r": "\r",
        "t": "\t",
        '"': '"',
        "\\": "\\",
        "$": "$",
        "`": "`",
    }
    if char in replacements:
        return replacements[char]
    return f"\\{char}"


def _consume_trailing_comment(
    text: str, pos: int, line_no: int
) -> tuple[int, int]:
    while pos < len(text) and text[pos] in " \t":
        pos += 1
    if pos < len(text) and text[pos] == "#":
        newline = text.find("\n", pos)
        if newline == -1:
            return len(text), line_no
        return newline + 1, line_no + 1
    if pos < len(text) and text[pos] == "\n":
        return pos + 1, line_no + 1
    if pos < len(text):
        raise DotenvParseError(
            f"Unexpected characters after quoted value at line {line_no}"
        )
    return pos, line_no


def _parse_unquoted_value(
    text: str, pos: int, line_no: int
) -> tuple[str, int, int]:
    newline = text.find("\n", pos)
    if newline == -1:
        raw = text[pos:]
        pos = len(text)
    else:
        raw = text[pos:newline]
        pos = newline + 1
        line_no += 1

    value = raw.strip()
    for index, char in enumerate(value):
        if char == "#" and (index == 0 or value[index - 1] in " \t"):
            value = value[:index].rstrip()
            break
    return value, pos, line_no


def serialize_dotenv_values(values: Mapping[str, str]) -> str:
    lines = []
    for key, value in values.items():
        escaped = value.replace("\\", "\\\\").replace('"', '\\"').replace("\r", "\\r")
        lines.append(f'{key}="{escaped}"')
    return "\n".join(lines) + "\n"


def verify_dotenv_round_trip(values: Mapping[str, str]) -> None:
    reparsed = parse_dotenv_text(serialize_dotenv_values(values))
    if dict(values) != reparsed:
        raise DotenvParseError("Dotenv parser round-trip check failed")


def gh_secret_names(repo: str) -> set[str]:
    result = run_gh(
        ("secret", "list", "--repo", repo, "--json", "name", "--jq", ".[].name")
    )
    return {line.strip() for line in result.stdout.splitlines() if line.strip()}


def run_gh(
    args: Sequence[str], input_text: str | None = None
) -> subprocess.CompletedProcess:
    try:
        result = subprocess.run(
            ("gh", *args),
            input=input_text,
            text=True,
            capture_output=True,
            check=False,
        )
    except FileNotFoundError as exc:
        raise RuntimeError("gh CLI not found on PATH") from exc

    if result.returncode != 0:
        stderr = result.stderr.strip() or result.stdout.strip() or "no gh output"
        raise RuntimeError(f"gh {' '.join(args)} failed: {stderr}")
    return result


def build_plan(
    env_values: Mapping[str, str], existing_names: set[str]
) -> list[PlannedSecret]:
    plan: list[PlannedSecret] = []
    for spec in ALLOWLIST:
        if spec.name not in env_values:
            status = "skip missing-env"
        elif env_values[spec.name] == "":
            status = "skip empty"
        elif spec.name in existing_names:
            status = "update"
        else:
            status = "set"
        plan.append(PlannedSecret(spec.name, status, spec.consumers))
    return plan


def print_plan(plan: Sequence[PlannedSecret], repo: str, env_file: Path) -> None:
    print(f"target repo: {repo}")
    print(f"env file: {env_file}")
    print("plan:")
    for item in plan:
        consumers = ", ".join(item.consumers)
        print(f"  {item.status:<16} {item.name} ({consumers})")


def apply_plan(
    plan: Sequence[PlannedSecret], env_values: Mapping[str, str], repo: str
) -> None:
    for item in plan:
        if item.status not in {"set", "update"}:
            print(f"{item.status.upper()} {item.name}")
            continue

        run_gh(
            ("secret", "set", item.name, "--repo", repo),
            input_text=env_values[item.name],
        )
        print(f"{item.status.upper()} {item.name}")


def scan_secret_refs_from_text(text: str) -> set[str]:
    refs: set[str] = set()
    for match in SECRET_REF_RE.finditer(text):
        refs.add(match.group(1) or match.group(2))
    return refs


def scan_workflow_secret_refs(repo_root: Path) -> set[str]:
    refs: set[str] = set()
    for relative_path in RELEASE_WORKFLOW_FILES:
        workflow_path = repo_root / relative_path
        refs.update(
            scan_secret_refs_from_text(workflow_path.read_text(encoding="utf-8"))
        )
    return refs


def print_check(repo: str, repo_root: Path) -> int:
    referenced = scan_workflow_secret_refs(repo_root)
    existing = gh_secret_names(repo)
    result = build_check_result(referenced, existing)

    print(f"target repo: {repo}")
    _print_name_group("present", result.present)
    _print_name_group("automatic", result.automatic)
    _print_name_group("missing external", result.external)
    _print_name_group("missing optional", result.optional)
    _print_name_group("missing required", result.missing_required)
    return 1 if result.missing_required else 0


def build_check_result(referenced: set[str], existing: set[str]) -> CheckResult:
    missing = referenced - existing
    return CheckResult(
        present=sorted(referenced & existing),
        automatic=sorted(missing & KNOWN_AUTOMATIC_SECRETS),
        external=sorted(missing & KNOWN_EXTERNAL_SECRETS),
        optional=sorted(missing & KNOWN_OPTIONAL_SECRETS),
        missing_required=sorted(
            missing
            - KNOWN_AUTOMATIC_SECRETS
            - KNOWN_EXTERNAL_SECRETS
            - KNOWN_OPTIONAL_SECRETS
        ),
    )


def _print_name_group(label: str, names: Sequence[str]) -> None:
    print(f"{label}:")
    if not names:
        print("  (none)")
        return
    for name in names:
        print(f"  {name}")


def load_env_for_sync(env_file: Path) -> dict[str, str]:
    if not env_file.exists():
        raise FileNotFoundError(f"env file not found: {env_file}")
    env_values = parse_dotenv_file(env_file)
    verify_dotenv_round_trip(env_values)
    return env_values


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Sync allowlisted release workflow secrets from .env.production."
    )
    parser.add_argument(
        "--repo", default=DEFAULT_REPO, help=f"target repo (default: {DEFAULT_REPO})"
    )
    parser.add_argument(
        "--env-file",
        type=Path,
        default=DEFAULT_ENV_FILE,
        help=f"dotenv file for dry-run/apply (default: {DEFAULT_ENV_FILE})",
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=REPO_ROOT,
        help="repo root used by --check workflow scanning",
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--dry-run", action="store_const", const="dry-run", dest="mode")
    mode.add_argument("--apply", action="store_const", const="apply", dest="mode")
    mode.add_argument("--check", action="store_const", const="check", dest="mode")
    parser.set_defaults(mode="dry-run")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    try:
        if args.mode == "check":
            return print_check(args.repo, args.repo_root)

        env_values = load_env_for_sync(args.env_file)
        existing_names = gh_secret_names(args.repo)
        plan = build_plan(env_values, existing_names)
        print_plan(plan, args.repo, args.env_file)
        if args.mode == "apply":
            apply_plan(plan, env_values, args.repo)
        return 0
    except (DotenvParseError, FileNotFoundError, RuntimeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
