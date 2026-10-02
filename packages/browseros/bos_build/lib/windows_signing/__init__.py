"""One signing boundary for browser packaging and server OTA.

Providers mutate only staged copies. The entire batch must verify before any
file replaces its input, so an authentication or trust failure stops packaging.
"""

import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import uuid
from pathlib import Path

from ..env import EnvConfig
from ..utils import IS_WINDOWS, log_error, log_info, redact_sensitive_text
from .config import SigningProvider, signing_identity, validate_signing


def file_digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def _verify(path: Path, provider: SigningProvider, env: EnvConfig) -> dict:
    escaped = str(path).replace("'", "''")
    script = f"""$ErrorActionPreference = 'Stop'
$s = Get-AuthenticodeSignature -LiteralPath '{escaped}'
if ($s.Status -ne 'Valid' -or -not $s.TimeStamperCertificate) {{
    throw 'A valid timestamped Authenticode signature is required'
}}
$eku = @($s.SignerCertificate.Extensions | Where-Object {{$_.Oid.Value -eq '2.5.29.37'}} | ForEach-Object {{ $_.EnhancedKeyUsages | ForEach-Object {{$_.Value}} }})
@{{status=[string]$s.Status; subject=$s.SignerCertificate.Subject;
   publisher=$s.SignerCertificate.GetNameInfo([System.Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false);
   issuer=$s.SignerCertificate.Issuer; thumbprint=$s.SignerCertificate.Thumbprint;
   eku=$eku; timestamp_subject=$s.TimeStamperCertificate.Subject;
   timestamp_thumbprint=$s.TimeStamperCertificate.Thumbprint}} | ConvertTo-Json -Compress
"""
    result = subprocess.run(
        ["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
        capture_output=True,
        text=True,
        timeout=120,
    )
    if result.returncode:
        raise RuntimeError(f"Authenticode verification failed for {path.name}")
    evidence = json.loads(result.stdout)
    if (
        evidence.get("status") != "Valid"
        or evidence.get("publisher") != env.windows_signing_publisher
    ):
        raise RuntimeError(
            f"Unexpected signing publisher or trust status for {path.name}"
        )
    if provider == SigningProvider.AZURE:
        if env.azure_signing_profile_oid not in evidence.get("eku", []):
            raise RuntimeError(f"Azure profile identity mismatch for {path.name}")
        # /all verifies every signature; /tw also reports a missing timestamp.
        result = subprocess.run(
            [
                os.environ["AZURE_SIGNTOOL_PATH"],
                "verify",
                "/pa",
                "/all",
                "/v",
                "/tw",
                str(path),
            ],
            capture_output=True,
            text=True,
            timeout=120,
        )
        if result.returncode:
            raise RuntimeError(
                f"SignTool chain/timestamp verification failed for {path.name}"
            )
        evidence["signtool_verification"] = result.stdout.strip()
    elif "SSL.com" not in evidence.get("issuer", ""):
        raise RuntimeError(f"Unexpected SSL.com issuer for {path.name}")
    return evidence


def sign_windows_files(
    files: list[Path], env: EnvConfig, provider: SigningProvider
) -> bool:
    """Return success only after signing, trust checks and evidence persistence."""
    try:
        provider = SigningProvider(provider)
        if not IS_WINDOWS():
            raise RuntimeError("Release signing and verification require Windows")
        validate_signing(provider, env)
        if not files or any(not path.is_file() for path in files):
            raise ValueError("Signing requires a non-empty list of existing files")
        log_info(f"Signing {len(files)} Windows file(s) with {provider}")
        records = []
        with tempfile.TemporaryDirectory(prefix="browseros-signing-") as name:
            temp_dir = Path(name)
            staged = []
            for i, original in enumerate(files):
                destination = temp_dir / str(i) / original.name
                destination.parent.mkdir()
                shutil.copy2(original, destination)
                staged.append(destination)
            if provider == SigningProvider.AZURE:
                from .azure import sign_azure

                sign_azure(staged, env, temp_dir)
            else:
                from .sslcom import sign_with_codesigntool

                if not sign_with_codesigntool(staged, env):
                    raise RuntimeError("SSL.com signing failed")
            for original, signed in zip(files, staged, strict=True):
                records.append(
                    {
                        "path": str(original.resolve()),
                        "before_sha256": file_digest(original),
                        "after_sha256": file_digest(signed),
                        **_verify(signed, provider, env),
                    }
                )
            # Evidence and signed files are a single release gate: failure to
            # persist the report is fatal, even when the provider returned 200.
            report = {
                "schema": "browseros-windows-signing-v1",
                "identity": signing_identity(provider, env),
                "source_sha": os.environ.get("RELEASE_SHA")
                or os.environ.get("BROWSEROS_BUILD_SOURCE_SHA")
                or os.environ.get("GITHUB_SHA", ""),
                "run_id": os.environ.get("GITHUB_RUN_ID", "local"),
                "attempt": os.environ.get("GITHUB_RUN_ATTEMPT", ""),
                "product": os.environ.get("PRODUCT", ""),
                "version": os.environ.get("VERSION", ""),
                "files": records,
            }
            report_dir = Path(
                os.environ.get(
                    "WINDOWS_SIGNING_REPORT_DIR",
                    str(files[0].parent / "signing-reports"),
                )
            )
            report_dir.mkdir(parents=True, exist_ok=True)
            report_path = report_dir / f"{provider}-{uuid.uuid4().hex}.json"
            report_path.write_text(
                json.dumps(report, indent=2) + "\n", encoding="utf-8"
            )
            for original, signed in zip(files, staged, strict=True):
                shutil.copy2(signed, original)
            log_info(f"Windows signing evidence: {report_path}")
        return True
    except Exception as error:
        log_error(redact_sensitive_text(f"Windows signing failed: {error}"))
        return False
