#!/usr/bin/env python3
"""Create immutable inputs for one full browser release."""

import argparse
import hashlib
import json
import re
import time
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Callable, Iterable

from ..lib.env import EnvConfig
from ..lib.r2 import get_r2_client
from .feeds.render import GUPDATE_NS, render_update_manifest
from .feeds.spec import EXTENSIONS, extension_by_name
from .resource_pins import (
    COMPONENT_RESOURCE_FAMILY,
    RESOURCE_FAMILIES,
    ResourcePin,
    resolve_resource_pins,
)


_SHA_RE = re.compile(r"[0-9a-f]{40}")
_RUN_ID_RE = re.compile(r"[1-9][0-9]*")
_BROWSER_VERSION_RE = re.compile(r"[0-9]+(?:\.[0-9]+){2,3}")
_SERVER_VERSION_RE = re.compile(r"[0-9]+(?:\.[0-9]+){2}")
_EXTENSION_VERSION_RE = re.compile(r"[0-9]+(?:\.[0-9]+){3}")
_HTTPS_ORIGIN_RE = re.compile(
    r"https://[A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?(?::[0-9]{1,5})?/?"
)
_COMPONENTS = {
    "browseros": frozenset(
        {"browseros-server", "agent-extension", "browseros-onboard"}
    ),
    "browserclaw": frozenset(
        {
            "browserclaw-server",
            "browserclaw-onboard",
            "browserclaw-extension",
        }
    ),
}
_EXTENSION_COMPONENT = {
    "agent": "agent-extension",
    "browserclaw": "browserclaw-extension",
}
_COMPONENT_TAG = {
    "agent-extension": "ext-agent/v{version}",
    "browserclaw-extension": "ext-browserclaw/v{version}",
    "browseros-onboard": "app-onboard/v{version}",
    "browserclaw-onboard": "claw-onboard/v{version}",
    "browserclaw-server": "claw-server/v{version}",
    "browseros-server": "agent-server/v{version}",
}


@dataclass(frozen=True)
class Component:
    """One prepared release component."""

    name: str
    version: str
    tag: str
    source_sha: str


@dataclass(frozen=True)
class PlanResult:
    """Paths and URLs emitted for downstream browser jobs."""

    plan_path: Path
    plan_key: str
    plan_url: str
    manifest_path: Path | None
    manifest_key: str
    manifest_url: str
    resource_versions: dict[str, str]


def _sha256(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()


def _read_body(response: dict, key: str) -> bytes:
    body = response.get("Body")
    if body is None or not hasattr(body, "read"):
        raise RuntimeError(f"R2 object {key} returned no readable body")
    try:
        return body.read()
    finally:
        close = getattr(body, "close", None)
        if close:
            close()


def _verify_r2_object(
    client,
    bucket: str,
    key: str,
    expected: bytes,
    binding: dict[str, str],
) -> None:
    try:
        response = client.get_object(Bucket=bucket, Key=key)
        actual = _read_body(response, key)
    except Exception as error:
        raise RuntimeError(f"Failed to verify R2 object {key}: {error}") from error
    if actual != expected:
        raise RuntimeError(
            f"R2 object verification failed for {key}: "
            f"expected sha256 {_sha256(expected)}, got {_sha256(actual)}"
        )
    metadata = response.get("Metadata", {})
    mismatches = {
        name: {"expected": value, "actual": metadata.get(name)}
        for name, value in binding.items()
        if metadata.get(name) != value
    }
    if mismatches:
        raise RuntimeError(f"R2 object binding mismatch for {key}: {mismatches}")


def _is_precondition_failure(error: Exception) -> bool:
    response = getattr(error, "response", {})
    code = str(response.get("Error", {}).get("Code", ""))
    status = response.get("ResponseMetadata", {}).get("HTTPStatusCode")
    return code in {
        "409",
        "412",
        "Conflict",
        "ConditionalRequestConflict",
        "PreconditionFailed",
    } or status in {409, 412}


def _upload_immutable(
    client,
    bucket: str,
    key: str,
    path: Path,
    binding: dict[str, str],
) -> None:
    content = path.read_bytes()
    expected_binding = {**binding, "sha256": _sha256(content)}
    try:
        client.put_object(
            Bucket=bucket,
            Key=key,
            Body=content,
            ContentType=(
                "application/json" if path.suffix == ".json" else "application/xml"
            ),
            Metadata=expected_binding,
            IfNoneMatch="*",
        )
    except Exception as error:
        if not _is_precondition_failure(error):
            raise RuntimeError(
                f"Failed to create immutable R2 object {key}: {error}"
            ) from error
        _verify_r2_object(client, bucket, key, content, expected_binding)
    _verify_r2_object(client, bucket, key, content, expected_binding)


def _default_http_head(url: str) -> int:
    status = 0
    for attempt in range(6):
        request = urllib.request.Request(url, method="HEAD")
        try:
            with urllib.request.urlopen(request, timeout=20) as response:
                status = response.status
        except urllib.error.HTTPError as error:
            status = error.code
        except urllib.error.URLError:
            status = 0
        if status == 200:
            return status
        if attempt < 5:
            time.sleep(2**attempt)
    return status


def _verify_url(url: str, http_head: Callable[[str], int]) -> None:
    status = http_head(url)
    if status != 200:
        raise RuntimeError(f"URL verification failed with HTTP {status}: {url}")


def _validated_components(
    product: str,
    source_sha: str,
    components: Iterable[Component],
) -> list[Component]:
    allowed = _COMPONENTS.get(product)
    if allowed is None:
        raise ValueError(f"unknown product: {product}")

    selected = list(components)
    names = [component.name for component in selected]
    duplicates = sorted({name for name in names if names.count(name) > 1})
    if duplicates:
        raise ValueError(f"duplicate components: {', '.join(duplicates)}")

    for component in selected:
        if component.name not in allowed:
            raise ValueError(f"unknown component for {product}: {component.name}")
        version_pattern = (
            _EXTENSION_VERSION_RE
            if component.name.endswith("-extension")
            else _SERVER_VERSION_RE
        )
        if not version_pattern.fullmatch(component.version):
            raise ValueError(
                f"invalid version for {component.name}: {component.version}"
            )
        expected_tag = _COMPONENT_TAG[component.name].format(version=component.version)
        if component.tag != expected_tag:
            raise ValueError(
                f"invalid tag for {component.name}: "
                f"expected {expected_tag}, got {component.tag}"
            )
        if component.source_sha != source_sha:
            raise ValueError(
                f"component source mismatch for {component.name}: "
                f"{component.source_sha} != {source_sha}"
            )
    return sorted(selected, key=lambda component: component.name)


def _local_name(tag: str) -> str:
    return tag.rsplit("}", maxsplit=1)[-1]


def _manifest_versions(content: str) -> dict[str, str]:
    try:
        root = ET.fromstring(content)
    except ET.ParseError as error:
        raise ValueError(f"invalid bundled manifest XML: {error}") from error
    if root.tag != f"{{{GUPDATE_NS}}}gupdate":
        raise ValueError("bundled manifest root must use the gupdate namespace")
    if root.attrib != {"protocol": "2.0"}:
        raise ValueError("bundled manifest root must contain only protocol=2.0")
    if root.text and root.text.strip():
        raise ValueError("bundled manifest root cannot contain mixed text")
    if any(element.tail and element.tail.strip() for element in root.iter()):
        raise ValueError("bundled manifest cannot contain mixed text")

    expected = {extension.extension_id: extension for extension in EXTENSIONS}
    versions: dict[str, str] = {}
    root_children = list(root)
    unexpected = [child for child in root_children if _local_name(child.tag) != "app"]
    if unexpected:
        raise ValueError(
            "unexpected bundled manifest root child: "
            + ", ".join(_local_name(child.tag) for child in unexpected)
        )
    direct_apps = [child for child in root_children if _local_name(child.tag) == "app"]
    all_apps = [element for element in root.iter() if _local_name(element.tag) == "app"]
    if len(all_apps) != len(direct_apps):
        raise ValueError("nested bundled manifest app entries are not allowed")
    for app in direct_apps:
        if app.tag != f"{{{GUPDATE_NS}}}app" or set(app.attrib) != {"appid"}:
            raise ValueError("bundled manifest app must contain only appid")
        if app.text and app.text.strip():
            raise ValueError("bundled manifest app cannot contain mixed text")
        extension_id = app.get("appid", "")
        if extension_id not in expected:
            raise ValueError(
                f"unknown bundled manifest entry: {extension_id or '<blank>'}"
            )
        extension_name = expected[extension_id].name
        if extension_name in versions:
            raise ValueError(f"duplicate bundled manifest entry: {extension_id}")
        app_children = list(app)
        if (
            len(app_children) != 1
            or app_children[0].tag != f"{{{GUPDATE_NS}}}updatecheck"
        ):
            raise ValueError(
                f"bundled manifest entry {extension_id} must have one updatecheck"
            )
        updatecheck = app_children[0]
        if set(updatecheck.attrib) != {"codebase", "version"}:
            raise ValueError(
                f"bundled manifest updatecheck {extension_id} has unexpected attributes"
            )
        if list(updatecheck) or (updatecheck.text and updatecheck.text.strip()):
            raise ValueError(
                f"bundled manifest updatecheck {extension_id} has nested content"
            )
        version = updatecheck.get("version", "")
        codebase = updatecheck.get("codebase", "")
        if not _EXTENSION_VERSION_RE.fullmatch(version):
            raise ValueError(
                f"bundled manifest entry {extension_id} has invalid version"
            )
        if "latest" in codebase.lower():
            raise ValueError(
                f"bundled manifest entry {extension_id} must use a versioned CRX URL"
            )
        expected_url = expected[extension_id].crx_url(version)
        if codebase != expected_url:
            raise ValueError(
                f"bundled manifest entry {extension_id} has non-versioned or "
                f"unexpected codebase: {codebase}"
            )
        versions[extension_name] = version

    missing = sorted(
        extension.name for extension in EXTENSIONS if extension.name not in versions
    )
    if missing:
        raise ValueError(f"missing bundled manifest entries: {', '.join(missing)}")
    return versions


def _render_manifest(
    path: Path,
    extension_name: str,
    extension_version: str,
) -> bytes:
    versions = _manifest_versions(path.read_text(encoding="utf-8"))
    if extension_name:
        if extension_name not in _EXTENSION_COMPONENT:
            raise ValueError(f"unknown selected extension: {extension_name}")
        if not _EXTENSION_VERSION_RE.fullmatch(extension_version):
            raise ValueError(f"invalid selected extension version: {extension_version}")
        versions[extension_name] = extension_version
    elif extension_version:
        raise ValueError("extension version provided without a selected extension")
    rendered = render_update_manifest(versions).encode("utf-8")
    validated = _manifest_versions(rendered.decode("utf-8"))
    if validated != versions:
        raise RuntimeError("generated bundled manifest did not preserve exact pins")
    return rendered


def _validated_resource_pins(
    pins: dict[str, ResourcePin],
    prepared: dict[str, tuple[str, str]],
) -> dict[str, ResourcePin]:
    families = {family.name: family for family in RESOURCE_FAMILIES}
    if set(pins) != set(families):
        raise ValueError(
            "resource pins must contain exactly: " + ", ".join(sorted(families))
        )
    for name, family in families.items():
        pin = pins[name]
        if pin.name != name or not _SERVER_VERSION_RE.fullmatch(pin.version):
            raise ValueError(f"invalid resource pin for {name}")
        objects = {item.target: item for item in pin.objects}
        if set(objects) != set(family.targets) or len(objects) != len(pin.objects):
            raise ValueError(f"invalid resource targets for {name}")
        for target, item in objects.items():
            if (
                item.key != family.version_key(pin.version, target)
                or not item.etag
                or item.size < 0
                or (item.sha256 and not re.fullmatch(r"[0-9a-f]{64}", item.sha256))
                or (item.release_sha and not _SHA_RE.fullmatch(item.release_sha))
            ):
                raise ValueError(f"invalid resource object for {name}/{target}")
        prepared_identity = prepared.get(name)
        if prepared_identity is not None:
            version, release_sha = prepared_identity
            if pin.version != version or any(
                item.release_sha != release_sha for item in pin.objects
            ):
                raise ValueError(f"prepared resource pin mismatch for {name}")
    return pins


def create_release_plan(
    *,
    product: str,
    browser_version: str,
    source_sha: str,
    run_id: str,
    run_attempt: str,
    components: Iterable[Component],
    extension_name: str,
    bundled_manifest_path: Path | None,
    output_dir: Path,
    client,
    bucket: str,
    cdn_base_url: str,
    http_head: Callable[[str], int] = _default_http_head,
    resource_pins: dict[str, ResourcePin] | None = None,
) -> PlanResult:
    """Create, upload, and verify one release plan and manifest."""
    if not _BROWSER_VERSION_RE.fullmatch(browser_version):
        raise ValueError(f"invalid browser version: {browser_version}")
    if not _SHA_RE.fullmatch(source_sha):
        raise ValueError(f"invalid source SHA: {source_sha}")
    if not _RUN_ID_RE.fullmatch(run_id) or len(run_id) > 20:
        raise ValueError(f"invalid run id: {run_id}")
    if not _RUN_ID_RE.fullmatch(run_attempt) or len(run_attempt) > 10:
        raise ValueError(f"invalid run attempt: {run_attempt}")
    if any(ord(character) < 32 or ord(character) == 127 for character in cdn_base_url):
        raise ValueError("CDN base URL must not contain control characters")
    if "\\" in cdn_base_url or not _HTTPS_ORIGIN_RE.fullmatch(cdn_base_url):
        raise ValueError("CDN base URL must be a credential-free HTTPS origin")
    cdn = urllib.parse.urlsplit(cdn_base_url)
    if (
        cdn.scheme != "https"
        or not cdn.hostname
        or cdn.username is not None
        or cdn.password is not None
        or cdn.query
        or cdn.fragment
        or cdn.path not in {"", "/"}
    ):
        raise ValueError("CDN base URL must be a credential-free HTTPS origin")
    try:
        port = cdn.port
    except ValueError as error:
        raise ValueError("CDN base URL has an invalid port") from error
    cdn_base_url = f"https://{cdn.hostname.lower()}"
    if port is not None:
        cdn_base_url += f":{port}"

    selected = _validated_components(product, source_sha, components)
    by_name = {component.name: component for component in selected}
    extension_components = set(by_name).intersection(_EXTENSION_COMPONENT.values())
    if not extension_name and extension_components:
        raise ValueError(
            "extension component was provided while selected extension is empty"
        )
    if bundled_manifest_path is None:
        raise ValueError("a bundled manifest snapshot is required")

    prepared_resources = {
        COMPONENT_RESOURCE_FAMILY[component.name]: (
            component.version,
            component.source_sha,
        )
        for component in selected
        if component.name in COMPONENT_RESOURCE_FAMILY
    }
    if resource_pins is None:
        resource_pins = resolve_resource_pins(
            client,
            bucket,
            prepared_resources,
        )
    resource_pins = _validated_resource_pins(resource_pins, prepared_resources)
    resource_versions = {
        name: resource_pins[name].version for name in sorted(resource_pins)
    }

    prefix = f"release-plans/{product}/{source_sha}/run-{run_id}-attempt-{run_attempt}"
    if "latest" in prefix.lower():
        raise ValueError("release plan key cannot contain latest")
    plan_key = f"{prefix}/release-plan.json"
    manifest_key = f"{prefix}/bundled-manifest.xml"
    manifest_url = f"{cdn_base_url}/{manifest_key}"
    selected_extension_version = ""

    if extension_name:
        component_name = _EXTENSION_COMPONENT.get(extension_name)
        component = by_name.get(component_name or "")
        if component is None:
            raise ValueError(
                f"selected extension {extension_name} has no prepared component"
            )
        selected_extension_version = component.version

    manifest_bytes = _render_manifest(
        bundled_manifest_path,
        extension_name,
        selected_extension_version,
    )
    manifest_versions = _manifest_versions(manifest_bytes.decode("utf-8"))
    for name, version in manifest_versions.items():
        _verify_url(extension_by_name(name).crx_url(version), http_head)

    output_dir.mkdir(parents=True, exist_ok=True)
    manifest_path = output_dir / "bundled-manifest.xml"
    manifest_path.write_bytes(manifest_bytes)

    plan = {
        "browser_version": browser_version,
        "components": [asdict(component) for component in selected],
        "objects": {
            "bundled_manifest": {
                "key": manifest_key,
                "sha256": _sha256(manifest_bytes),
                "url": manifest_url,
            },
            "release_plan": {
                "key": plan_key,
                "url": f"{cdn_base_url}/{plan_key}",
            },
        },
        "product": product,
        "resource_pins": {
            name: resource_pins[name].to_dict() for name in sorted(resource_pins)
        },
        "schema_version": 2,
        "source_sha": source_sha,
        "workflow_run_attempt": int(run_attempt),
        "workflow_run_id": run_id,
    }
    plan_bytes = (json.dumps(plan, indent=2, sort_keys=True) + "\n").encode()
    plan_path = output_dir / "release-plan.json"
    plan_path.write_bytes(plan_bytes)

    binding = {
        "object-schema": "browseros-release-plan-v2",
        "plan-key": plan_key,
        "product": product,
        "run-attempt": run_attempt,
        "run-id": run_id,
        "source-sha": source_sha,
    }
    _upload_immutable(
        client,
        bucket,
        manifest_key,
        manifest_path,
        {**binding, "object-kind": "bundled-manifest"},
    )
    _upload_immutable(
        client,
        bucket,
        plan_key,
        plan_path,
        {**binding, "object-kind": "release-plan"},
    )

    plan_url = f"{cdn_base_url}/{plan_key}"
    _verify_url(manifest_url, http_head)
    _verify_url(plan_url, http_head)
    return PlanResult(
        plan_path=plan_path,
        plan_key=plan_key,
        plan_url=plan_url,
        manifest_path=manifest_path,
        manifest_key=manifest_key,
        manifest_url=manifest_url,
        resource_versions=resource_versions,
    )


def _components_json(value: str) -> list[Component]:
    try:
        raw = json.loads(value)
    except json.JSONDecodeError as error:
        raise ValueError(f"invalid components JSON: {error}") from error
    if not isinstance(raw, list):
        raise ValueError("components JSON must be a list")
    components = []
    expected = {"name", "version", "tag", "source_sha"}
    for index, item in enumerate(raw):
        if not isinstance(item, dict) or set(item) != expected:
            raise ValueError(
                f"component {index} must contain exactly: "
                + ", ".join(sorted(expected))
            )
        if not all(isinstance(value, str) for value in item.values()):
            raise ValueError(f"component {index} fields must be strings")
        components.append(Component(**item))
    return components


def _write_github_output(path: Path, result: PlanResult) -> None:
    values = {
        "manifest_key": result.manifest_key,
        "manifest_path": str(result.manifest_path or ""),
        "manifest_url": result.manifest_url,
        "plan_key": result.plan_key,
        "plan_path": str(result.plan_path),
        "plan_url": result.plan_url,
        **{
            f"{name}_version": version
            for name, version in result.resource_versions.items()
        },
    }
    with path.open("a", encoding="utf-8") as output:
        for name, value in values.items():
            if "\r" in value or "\n" in value:
                raise ValueError(f"unsafe output value for {name}")
            output.write(f"{name}={value}\n")


def main() -> None:
    """Run the release-plan CLI."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--product", required=True, choices=sorted(_COMPONENTS))
    parser.add_argument("--browser-version", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--run-attempt", required=True)
    parser.add_argument("--components-json", required=True)
    parser.add_argument("--extension-name", default="")
    parser.add_argument("--bundled-manifest", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--cdn-base-url", default="https://cdn.browseros.com")
    args = parser.parse_args()

    env = EnvConfig()
    if not env.has_r2_config():
        raise RuntimeError("R2 configuration not set")
    client = get_r2_client(env)
    if client is None:
        raise RuntimeError("Failed to create R2 client")

    result = create_release_plan(
        product=args.product,
        browser_version=args.browser_version,
        source_sha=args.source_sha,
        run_id=args.run_id,
        run_attempt=args.run_attempt,
        components=_components_json(args.components_json),
        extension_name=args.extension_name,
        bundled_manifest_path=args.bundled_manifest,
        output_dir=args.output_dir,
        client=client,
        bucket=env.r2_bucket,
        cdn_base_url=args.cdn_base_url,
    )
    if args.github_output:
        _write_github_output(args.github_output, result)
    print(result.plan_url)


if __name__ == "__main__":
    main()
