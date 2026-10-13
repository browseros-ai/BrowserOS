"""Azure Public Trust adapter with fresh, short-lived GitHub authentication.

Compilation can outlive an Azure login. Obtain an assertion at each signing
batch and expose it only to Dlib through a temporary workload-identity file.
"""

import json
import os
import subprocess
from pathlib import Path
from urllib.parse import parse_qsl, urlencode, urlparse, urlunparse

import requests

from ..env import EnvConfig, SENSITIVE_ENV_VARS
from ..utils import log_error, redact_sensitive_text

_CREDENTIALS = (
    "EnvironmentCredential",
    "WorkloadIdentityCredential",
    "ManagedIdentityCredential",
    "SharedTokenCacheCredential",
    "VisualStudioCredential",
    "VisualStudioCodeCredential",
    "AzureCliCredential",
    "AzurePowerShellCredential",
    "AzureDeveloperCliCredential",
    "InteractiveBrowserCredential",
)
TIMESTAMP_URL = "http://timestamp.acs.microsoft.com"


def _github_assertion() -> str:
    url = urlparse(os.environ["ACTIONS_ID_TOKEN_REQUEST_URL"])
    if url.scheme != "https" or not (url.hostname or "").endswith(
        ".actions.githubusercontent.com"
    ):
        raise ValueError("Unexpected GitHub OIDC endpoint")
    query = [(k, v) for k, v in parse_qsl(url.query) if k != "audience"]
    url = url._replace(
        query=urlencode([*query, ("audience", "api://AzureADTokenExchange")])
    )
    response = requests.get(
        urlunparse(url),
        timeout=30,
        allow_redirects=False,
        headers={
            "Authorization": "Bearer " + os.environ["ACTIONS_ID_TOKEN_REQUEST_TOKEN"]
        },
    )
    if response.status_code != 200:
        raise RuntimeError(f"GitHub OIDC request failed (HTTP {response.status_code})")
    assertion = response.json().get("value")
    if not isinstance(assertion, str) or assertion.count(".") != 2:
        raise RuntimeError("GitHub did not return an OIDC assertion")
    return assertion


def sign_azure(files: list[Path], env: EnvConfig, temp_dir: Path) -> None:
    """Sign staged files, letting the facade own verification and publication."""
    selected = (
        "WorkloadIdentityCredential"
        if env.azure_signing_auth == "github-oidc"
        else "AzureCliCredential"
    )
    assertion = ""
    child_env = dict(os.environ)
    # Do not let an unrelated machine credential win DefaultAzureCredential's
    # chain, or leak an old token file into the explicitly selected auth mode.
    for name in (
        *SENSITIVE_ENV_VARS,
        "AZURE_CLIENT_SECRET",
        "AZURE_CLIENT_CERTIFICATE_PATH",
        "AZURE_FEDERATED_TOKEN_FILE",
        "AZURE_TOKEN_CREDENTIALS",
    ):
        child_env.pop(name, None)
    if selected == "WorkloadIdentityCredential":
        assertion = _github_assertion()
        token_file = temp_dir / "github-oidc.jwt"
        fd = os.open(token_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            stream.write(assertion)
        child_env["AZURE_FEDERATED_TOKEN_FILE"] = str(token_file)
    metadata = {
        "Endpoint": env.azure_signing_endpoint,
        "CodeSigningAccountName": env.azure_signing_account,
        "CertificateProfileName": env.azure_signing_profile,
        "CorrelationId": "/".join(
            os.environ.get(n, "local") for n in ("GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT")
        ),
        "ExcludeCredentials": [name for name in _CREDENTIALS if name != selected],
    }
    metadata_file = temp_dir / "azure-metadata.json"
    metadata_file.write_text(json.dumps(metadata), encoding="utf-8")
    cmd = [
        os.environ["AZURE_SIGNTOOL_PATH"],
        "sign",
        "/v",
        "/fd",
        "SHA256",
        "/tr",
        TIMESTAMP_URL,
        "/td",
        "SHA256",
        "/dlib",
        os.environ["AZURE_SIGNING_DLIB_PATH"],
        "/dmdf",
        str(metadata_file),
        *map(str, files),
    ]
    result = subprocess.run(
        cmd, env=child_env, capture_output=True, text=True, timeout=600
    )
    if result.returncode:
        log_error(redact_sensitive_text(result.stdout + result.stderr, (assertion,)))
        raise RuntimeError(f"Azure SignTool failed (exit {result.returncode})")
