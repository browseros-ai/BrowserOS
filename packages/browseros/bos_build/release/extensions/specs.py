#!/usr/bin/env python3
"""Extension packaging spec table — one owner for how each extension is
built, signed and shipped to the CDN.

Same descriptor pattern as feeds/spec.py: a frozen dataclass plus a
module-level table. This absorbs the actions repo's configs/extensions.yaml;
the correction over that file is source fidelity — agent and browserclaw
live in THIS monorepo, so they build from the working tree instead of
cloning the repo they are already in. Extension ids are single-sourced from
core/products.py; specs_test pins the crx key formula to feeds/spec.py so
the two tables cannot drift.
"""

from dataclasses import dataclass
from typing import Optional, Tuple, Union

from ...core.products import (
    BROWSEROS_AGENT_EXTENSION_ID,
    BROWSEROS_BUG_REPORTER_EXTENSION_ID,
    BROWSEROS_CONTROLLER_EXTENSION_ID,
    BROWSERCLAW_EXTENSION_ID,
)


@dataclass(frozen=True)
class InRepoSource:
    """Extension source living in this monorepo; path is monorepo-relative."""

    path: str


@dataclass(frozen=True)
class ExternalRepoSource:
    """Extension source cloned from GitHub (owner/repo) at a default branch."""

    repo: str
    branch: str


@dataclass(frozen=True)
class ExtensionSpec:
    """One packagable extension; name is the crx filename prefix.

    dist_path / manifest_path / env_dir are relative to the resolved source
    root. env lists the variable names the build reads; required_env is the
    subset that must be present before production builds. Values are written
    to <env_dir or source root>/.env because some bundler configs only read
    env from file.
    """

    name: str
    source: Union[InRepoSource, ExternalRepoSource]
    pre_build: str
    build: str
    dist_path: str
    manifest_path: str
    extension_id: str
    signing_key_env: str
    env: Tuple[str, ...] = ()
    required_env: Tuple[str, ...] = ()
    env_dir: str = ""

    def crx_filename(self, version: str) -> str:
        return f"{self.name}-{version}.crx"

    def crx_key(self, version: str) -> str:
        return f"extensions/{self.crx_filename(version)}"


EXTENSION_SPECS: Tuple[ExtensionSpec, ...] = (
    ExtensionSpec(
        name="agent",
        source=InRepoSource(path="packages/browseros-agent"),
        # bun ci (frozen lockfile) for in-repo builds: a release must not
        # resolve fresh deps or mutate the working-tree lockfile.
        pre_build="bun ci",
        build="bun run build:agent",
        dist_path="apps/app/dist/chrome-mv3",
        manifest_path="apps/app/package.json",
        extension_id=BROWSEROS_AGENT_EXTENSION_ID,
        signing_key_env="BROWSEROS_AGENT_V2_KEY",
        env=(
            "VITE_PUBLIC_SENTRY_DSN",
            "SENTRY_AUTH_TOKEN",
            "SENTRY_ORG",
            "SENTRY_PROJECT",
            "VITE_PUBLIC_POSTHOG_KEY",
            "VITE_PUBLIC_POSTHOG_HOST",
            "VITE_PUBLIC_BROWSEROS_API",
            "GRAPHQL_SCHEMA_PATH",
            "NODE_ENV",
        ),
        env_dir="apps/app",
    ),
    ExtensionSpec(
        name="controller",
        source=ExternalRepoSource(repo="browseros-ai/BrowserOS-agent", branch="main"),
        pre_build="bun install",
        build="bun run build:ext",
        dist_path="apps/controller-ext/dist",
        manifest_path="apps/controller-ext/manifest.json",
        extension_id=BROWSEROS_CONTROLLER_EXTENSION_ID,
        signing_key_env="BROWSEROS_CONTROLLER_KEY",
        env=("NODE_ENV", "POSTHOG_API_KEY"),
    ),
    ExtensionSpec(
        name="bugreporter",
        source=ExternalRepoSource(
            repo="browseros-ai/BrowserOS-feedback-extension", branch="main"
        ),
        # --production=false so devDependencies (rimraf et al.) install too.
        pre_build="yarn install --production=false",
        build="yarn run build",
        dist_path="dist",
        manifest_path="manifest.json",
        extension_id=BROWSEROS_BUG_REPORTER_EXTENSION_ID,
        signing_key_env="BUGREPORTER_KEY",
        env=("NODE_ENV",),
    ),
    ExtensionSpec(
        name="browserclaw",
        source=InRepoSource(path="packages/browseros-agent"),
        pre_build="bun ci",
        build="bun run --filter @browseros/claw-app build",
        dist_path="apps/claw-app/dist/chrome-mv3",
        manifest_path="apps/claw-app/package.json",
        extension_id=BROWSERCLAW_EXTENSION_ID,
        signing_key_env="BROWSERCLAW_KEY",
        env=(
            "VITE_CLAW_POSTHOG_KEY",
            "VITE_CLAW_POSTHOG_HOST",
            "NODE_ENV",
        ),
        required_env=("VITE_CLAW_POSTHOG_KEY",),
        env_dir="apps/claw-app",
    ),
)


def spec_by_name(name: str) -> ExtensionSpec:
    for spec in EXTENSION_SPECS:
        if spec.name == name:
            return spec
    valid = ", ".join(sorted(spec.name for spec in EXTENSION_SPECS))
    raise ValueError(f"Unknown extension '{name}'. Valid: {valid}")


def select_specs(name: Optional[str]) -> Tuple[ExtensionSpec, ...]:
    """Specs for one --name, or the whole table when name is None."""
    if name is None:
        return EXTENSION_SPECS
    return (spec_by_name(name),)
