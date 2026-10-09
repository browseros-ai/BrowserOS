"""Resolve provider identity and prerequisites before expensive release work."""

import os
import hashlib
import re
from enum import StrEnum
from pathlib import Path
from urllib.parse import urlparse

from ..env import EnvConfig


class SigningProvider(StrEnum):
    """Explicit release choices; a signing failure never changes providers."""

    SSLCOM = "sslcom"
    AZURE = "azure"


def browser_providers(env: EnvConfig) -> tuple[SigningProvider, SigningProvider]:
    return env.windows_signing_provider, env.windows_server_signing_provider


def required_signing_env(providers, env: EnvConfig) -> tuple[str, ...]:
    required: list[str] = []
    for provider in dict.fromkeys(providers):
        if SigningProvider(provider) == SigningProvider.SSLCOM:
            required.extend(
                [
                    "CODE_SIGN_TOOL_EXE"
                    if env.code_sign_tool_exe
                    else "CODE_SIGN_TOOL_PATH",
                    "ESIGNER_USERNAME",
                    "ESIGNER_PASSWORD",
                    "ESIGNER_TOTP_SECRET",
                ]
            )
        else:
            required.extend(
                [
                    "AZURE_SIGNING_ENDPOINT",
                    "AZURE_SIGNING_ACCOUNT",
                    "AZURE_SIGNING_PROFILE",
                    "AZURE_SIGNING_PROFILE_OID",
                    "AZURE_SIGNTOOL_PATH",
                    "AZURE_SIGNING_DLIB_PATH",
                ]
            )
            if env.azure_signing_auth == "github-oidc":
                required.extend(
                    [
                        "AZURE_TENANT_ID",
                        "AZURE_CLIENT_ID",
                        "ACTIONS_ID_TOKEN_REQUEST_URL",
                        "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
                    ]
                )
    return tuple(dict.fromkeys(required))


def signing_identity(provider: SigningProvider, env: EnvConfig) -> dict[str, str]:
    """Stable identity for checkpoints and payload reuse, never a rotating cert."""
    identity = {"provider": str(provider), "publisher": env.windows_signing_publisher}
    if provider == SigningProvider.AZURE:
        identity.update(
            {
                "endpoint": env.azure_signing_endpoint,
                "account": env.azure_signing_account,
                "profile": env.azure_signing_profile,
                "profile_oid": env.azure_signing_profile_oid,
            }
        )
    else:
        identity["credential_id_sha256"] = hashlib.sha256((env.esigner_credential_id or "").encode()).hexdigest()
    return identity


def browser_signing_identity(env: EnvConfig) -> dict:
    browser, server = browser_providers(env)
    return {
        "browser": signing_identity(browser, env),
        "server": signing_identity(server, env),
    }


def validate_signing(provider: SigningProvider, env: EnvConfig) -> None:
    provider = SigningProvider(provider)
    missing = [
        name
        for name in required_signing_env([provider], env)
        if not os.environ.get(name)
    ]
    if missing:
        raise ValueError(f"Missing {provider} signing settings: {', '.join(missing)}")
    if not env.windows_signing_publisher.strip():
        raise ValueError("WINDOWS_SIGNING_PUBLISHER must identify the legal publisher")
    if provider == SigningProvider.SSLCOM:
        from .sslcom import check_signing_environment

        if not check_signing_environment(env):
            raise ValueError("Invalid SSL.com signing configuration")
        return
    endpoint = urlparse(env.azure_signing_endpoint)
    if (
        endpoint.scheme != "https"
        or not endpoint.hostname
        or not endpoint.hostname.endswith(".codesigning.azure.net")
        or endpoint.username
        or endpoint.password
        or endpoint.port
        or endpoint.path not in ("", "/")
        or endpoint.query
        or endpoint.fragment
    ):
        raise ValueError(
            "AZURE_SIGNING_ENDPOINT must be a regional Azure HTTPS endpoint"
        )
    if not re.fullmatch(r"[0-9]+(?:\.[0-9]+)+", env.azure_signing_profile_oid):
        raise ValueError(
            "AZURE_SIGNING_PROFILE_OID must be the Public Trust profile's EKU OID"
        )
    for name in ("AZURE_SIGNTOOL_PATH", "AZURE_SIGNING_DLIB_PATH"):
        if not Path(os.environ[name]).is_file():
            raise ValueError(f"Signing tool does not exist: {name}")
