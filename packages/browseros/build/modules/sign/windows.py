#!/usr/bin/env python3
"""
Optimized Windows signing module for BrowserOS
Based on industry best practices from Chromium, Electron, and Brave
"""

import subprocess
import time
import os
from pathlib import Path
from typing import List, Optional, Dict, Tuple
from dataclasses import dataclass
from enum import Enum

from ...common.module import CommandModule, ValidationError
from ...common.context import Context
from ...common.env import EnvConfig
from ...common.utils import (
    log_info,
    log_error,
    log_success,
    log_warning,
    join_paths,
    IS_WINDOWS,
)


class SigningStrategy(Enum):
    """Signing strategies available"""
    BATCH_SIGN = "batch_sign"          # Sign all files in batch
    BATCH_SIGN_HASH = "batch_sign_hash"  # Sign pre-computed hashes
    INDIVIDUAL = "individual"           # Sign files one by one
    HYBRID = "hybrid"                  # Try batch, fallback to individual


@dataclass
class SigningConfig:
    """Configuration for signing operation"""
    strategy: SigningStrategy = SigningStrategy.HYBRID
    batch_size: int = 100  # Max files per batch (SSL.com limit)
    enable_malware_scan: bool = False  # Disabled for faster signing
    retry_attempts: int = 3
    retry_delay: int = 5  # seconds
    timeout: int = 1800  # 30 minutes for batch operations
    verify_after_sign: bool = True


# Critical binaries that MUST be signed (order matters for priority)
CRITICAL_BINARIES = [
    "chrome.exe",              # Main Chrome executable
    "chrome_elf.dll",          # MOST CRITICAL - Often flagged by Windows Defender
    "chrome.dll",              # Main Chrome library
    "chrome_wer.dll",          # Windows Error Reporting
    "chrome_proxy.exe",        # Chrome proxy
    "chrome_pwa_launcher.exe", # PWA launcher
    "notification_helper.exe", # Notifications helper
    "elevation_service.exe",   # Elevation service
    "BrowserOS.exe",           # Main BrowserOS executable
]

# Additional binaries that should be signed
STANDARD_BINARIES = [
    "chromedriver.exe",
    "nacl64.exe",
    "software_reporter_tool.exe",
    "browseros_server.exe",
]

# File patterns to exclude from signing
EXCLUDE_PATTERNS = [
    "*test*.exe",
    "*test*.dll",
    "*mock*.dll",
    "*example*.exe",
]


class WindowsSignModule(CommandModule):
    """Optimized Windows signing module with multiple strategies"""
    
    produces = ["signed_installer"]
    requires = ["built_app"]
    description = "Sign Windows binaries and create signed installer"
    
    def __init__(self):
        super().__init__()
        self.config = SigningConfig()
        self.signed_files: List[Path] = []
        self.failed_files: List[Path] = []
        
    def validate(self, ctx: Context) -> None:
        """Validate environment and prerequisites"""
        if not IS_WINDOWS():
            raise ValidationError("Windows signing requires Windows")
        
        build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)
        if not build_output_dir.exists():
            raise ValidationError(f"Build output directory not found: {build_output_dir}")
        
        # Validate SSL.com credentials
        env = ctx.env
        self._validate_credentials(env)
        
        # Check time synchronization (critical for TOTP)
        if not self._check_time_sync():
            log_warning("System time may not be synchronized - TOTP authentication may fail")
    
    def execute(self, ctx: Context) -> None:
        """Execute signing process"""
        log_info("\n🔏 Starting optimized Windows signing process...")
        
        build_output_dir = join_paths(ctx.chromium_src, ctx.out_dir)
        
        # Step 1: Collect and categorize binaries
        binaries = self._collect_binaries(build_output_dir)
        log_info(f"Found {len(binaries)} binaries to sign")
        
        # Step 2: Sign binaries using configured strategy
        success = self._sign_binaries(binaries, ctx.env)
        
        if not success:
            raise RuntimeError("Failed to sign critical binaries")
        
        # Step 3: Verify critical signatures
        if self.config.verify_after_sign:
            if not self._verify_critical_signatures(build_output_dir):
                raise RuntimeError("Critical file signature verification failed")
        
        # Step 4: Build mini installer with signed binaries
        log_info("\nBuilding mini_installer with signed binaries...")
        if not self._build_mini_installer(ctx):
            raise RuntimeError("Failed to build mini_installer")
        
        # Step 5: Sign the installer
        mini_installer_path = build_output_dir / "mini_installer.exe"
        if not self._sign_file(mini_installer_path, ctx.env):
            raise RuntimeError("Failed to sign mini_installer.exe")
        
        ctx.artifact_registry.add("signed_installer", mini_installer_path)
        
        # Report summary
        self._report_summary()
        log_success("✅ Windows signing completed successfully!")
    
    def _collect_binaries(self, build_dir: Path) -> List[Path]:
        """Collect all binaries that need signing"""
        binaries = []
        seen = set()
        
        # First, collect critical binaries in order
        for name in CRITICAL_BINARIES:
            found = list(build_dir.rglob(name))
            for path in found:
                if path not in seen and self._should_sign(path):
                    binaries.append(path)
                    seen.add(path)
        
        # Then collect standard binaries
        for name in STANDARD_BINARIES:
            found = list(build_dir.rglob(name))
            for path in found:
                if path not in seen and self._should_sign(path):
                    binaries.append(path)
                    seen.add(path)
        
        # Finally, collect all other EXE and DLL files
        for pattern in ["*.exe", "*.dll"]:
            for path in build_dir.rglob(pattern):
                if path not in seen and self._should_sign(path):
                    binaries.append(path)
                    seen.add(path)
        
        return binaries
    
    def _should_sign(self, path: Path) -> bool:
        """Check if a file should be signed"""
        # Skip test files and other excluded patterns
        name = path.name.lower()
        for pattern in EXCLUDE_PATTERNS:
            if pattern.replace("*", "") in name:
                return False
        
        # Skip files in test/temp directories
        parts = path.parts
        skip_dirs = {"test", "tests", "testing", "temp", "tmp", "obj", "gen"}
        if any(d in skip_dirs for d in parts):
            return False
        
        # Skip PDB and lib files
        if path.suffix.lower() in [".pdb", ".lib", ".exp"]:
            return False
        
        return True
    
    def _sign_binaries(self, binaries: List[Path], env: EnvConfig) -> bool:
        """Sign binaries using configured strategy"""
        if self.config.strategy == SigningStrategy.BATCH_SIGN:
            return self._batch_sign_all(binaries, env)
        elif self.config.strategy == SigningStrategy.BATCH_SIGN_HASH:
            return self._batch_sign_hash_all(binaries, env)
        elif self.config.strategy == SigningStrategy.INDIVIDUAL:
            return self._sign_individually(binaries, env)
        else:  # HYBRID
            return self._sign_hybrid(binaries, env)
    
    def _sign_hybrid(self, binaries: List[Path], env: EnvConfig) -> bool:
        """Hybrid strategy: try batch first, fallback to individual"""
        log_info("Using hybrid signing strategy (batch with fallback)...")
        
        # Try batch signing first
        if self._batch_sign_all(binaries, env):
            return True
        
        # Fallback to individual signing
        log_warning("Batch signing failed, falling back to individual signing...")
        return self._sign_individually(binaries, env)
    
    def _batch_sign_all(self, binaries: List[Path], env: EnvConfig) -> bool:
        """Batch sign all binaries"""
        log_info(f"Attempting to batch sign {len(binaries)} files...")
        
        # Process in chunks of batch_size
        for i in range(0, len(binaries), self.config.batch_size):
            batch = binaries[i:i+self.config.batch_size]
            batch_num = (i // self.config.batch_size) + 1
            total_batches = (len(binaries) + self.config.batch_size - 1) // self.config.batch_size
            
            log_info(f"\nProcessing batch {batch_num}/{total_batches} ({len(batch)} files)...")
            
            # Create temp directory for this batch
            import tempfile
            with tempfile.TemporaryDirectory(prefix="sign_batch_") as temp_dir:
                temp_path = Path(temp_dir)
                input_dir = temp_path / "input"
                output_dir = temp_path / "output"
                input_dir.mkdir()
                output_dir.mkdir()
                
                # Copy files to input directory
                import shutil
                file_map = {}
                for binary in batch:
                    dest_name = f"{binary.parent.name}_{binary.name}" if len(batch) > 1 else binary.name
                    dest = input_dir / dest_name
                    shutil.copy2(binary, dest)
                    file_map[dest_name] = binary
                
                # Perform batch signing with retries
                success = self._batch_sign_with_retry(input_dir, output_dir, env)
                
                if success:
                    # Copy signed files back
                    for signed_file in output_dir.glob("*"):
                        if signed_file.name in file_map:
                            shutil.move(str(signed_file), str(file_map[signed_file.name]))
                            self.signed_files.append(file_map[signed_file.name])
                            log_success(f"✓ Signed: {file_map[signed_file.name].name}")
                else:
                    # Mark batch as failed
                    self.failed_files.extend(batch)
                    return False
        
        return len(self.failed_files) == 0
    
    def _batch_sign_with_retry(self, input_dir: Path, output_dir: Path, env: EnvConfig) -> bool:
        """Batch sign with retry logic"""
        for attempt in range(self.config.retry_attempts):
            if attempt > 0:
                log_info(f"Retry attempt {attempt}/{self.config.retry_attempts}...")
                time.sleep(self.config.retry_delay * attempt)  # Exponential backoff
            
            # Skip malware scan for performance (disabled by default)
            if self.config.enable_malware_scan:
                if not self._scan_directory(input_dir, env):
                    log_error("Malware scan failed")
                    continue
            
            # Attempt batch signing
            if self._execute_batch_sign(input_dir, output_dir, env):
                return True
        
        return False
    
    def _scan_directory(self, input_dir: Path, env: EnvConfig) -> bool:
        """Scan directory for malware before signing"""
        log_info("Scanning files for malware...")
        
        cmd = self._build_command("scan", env, {
            "-input_dir_path": str(input_dir)
        })
        
        result = self._run_command(cmd, env, timeout=600)
        
        if result.returncode == 0 and "malware" not in result.stdout.lower():
            log_success("✓ Malware scan passed")
            return True
        
        return False
    
    def _execute_batch_sign(self, input_dir: Path, output_dir: Path, env: EnvConfig) -> bool:
        """Execute batch_sign command"""
        params = {
            "-input_dir_path": str(input_dir),
            "-output_dir_path": str(output_dir)
        }
        
        # Build command without malware_block parameter to use server defaults
        cmd = self._build_command("batch_sign", env, params)
        
        result = self._run_command(cmd, env, timeout=self.config.timeout)
        
        # Check if signing was successful
        output_files = list(output_dir.glob("*"))
        input_files = list(input_dir.glob("*"))
        
        if result.returncode == 0 and len(output_files) == len(input_files):
            return True
        
        # Check for specific errors
        if "hash needs to be scanned first" in result.stdout:
            log_warning("Server requires malware scanning - retrying with individual signing")
            return False
        elif "Error:" in result.stdout or "Failed" in result.stdout:
            log_error("Batch signing failed")
        
        return False
    
    def _sign_individually(self, binaries: List[Path], env: EnvConfig) -> bool:
        """Sign files individually"""
        log_info(f"Signing {len(binaries)} files individually...")
        
        critical_bins = [b for b in binaries if b.name in CRITICAL_BINARIES]
        other_bins = [b for b in binaries if b.name not in CRITICAL_BINARIES]
        
        # Sign critical files first
        for binary in critical_bins:
            if not self._sign_file_with_retry(binary, env):
                log_error(f"Failed to sign critical file: {binary.name}")
                return False
        
        # Sign other files (continue on failure)
        for binary in other_bins:
            if not self._sign_file_with_retry(binary, env):
                log_warning(f"Failed to sign: {binary.name}")
                self.failed_files.append(binary)
            else:
                self.signed_files.append(binary)
        
        return True
    
    def _sign_file_with_retry(self, file_path: Path, env: EnvConfig) -> bool:
        """Sign a single file with retry logic"""
        for attempt in range(self.config.retry_attempts):
            if attempt > 0:
                time.sleep(self.config.retry_delay * attempt)
            
            if self._sign_file(file_path, env):
                return True
        
        return False
    
    def _sign_file(self, file_path: Path, env: EnvConfig) -> bool:
        """Sign a single file"""
        log_info(f"Signing {file_path.name}...")
        
        import tempfile
        with tempfile.TemporaryDirectory(prefix="sign_single_") as temp_dir:
            output_dir = Path(temp_dir)
            
            cmd = self._build_command("sign", env, {
                "-input_file_path": str(file_path),
                "-output_dir_path": str(output_dir),
                "-override": ""
            })
            
            result = self._run_command(cmd, env, timeout=300)
            
            if result.returncode == 0:
                # Check if signed file exists
                signed_file = output_dir / file_path.name
                if signed_file.exists():
                    import shutil
                    shutil.move(str(signed_file), str(file_path))
                    log_success(f"✓ Signed: {file_path.name}")
                    return True
                elif "Code signed successfully" in result.stdout:
                    # File was signed in place with -override
                    log_success(f"✓ Signed: {file_path.name}")
                    return True
        
        return False
    
    def _build_command(self, operation: str, env: EnvConfig, params: Dict[str, str]) -> List[str]:
        """Build CodeSignTool command"""
        tool_path = self._get_codesigntool_path(env)
        
        cmd = [str(tool_path), operation]
        
        # Add authentication parameters
        cmd.extend([
            "-username", env.esigner_username,
            "-password", f'"{env.esigner_password}"'
        ])
        
        if env.esigner_credential_id:
            cmd.extend(["-credential_id", env.esigner_credential_id])
        
        if env.esigner_totp_secret:
            cmd.extend(["-totp_secret", env.esigner_totp_secret])
        
        # Add operation-specific parameters
        for key, value in params.items():
            if value:
                cmd.extend([key, value])
            else:
                cmd.append(key)
        
        return cmd
    
    def _run_command(self, cmd: List[str], env: EnvConfig, timeout: int = 300) -> subprocess.CompletedProcess:
        """Run a command with proper error handling"""
        cmd_str = " ".join(cmd)
        tool_path = self._get_codesigntool_path(env)
        
        # Log command (hiding sensitive info)
        safe_cmd = cmd_str.replace(env.esigner_password, "***")
        safe_cmd = safe_cmd.replace(env.esigner_totp_secret or "", "***")
        log_info(f"Executing: {safe_cmd}")
        
        try:
            result = subprocess.run(
                cmd_str,
                shell=True,
                capture_output=True,
                text=True,
                cwd=str(tool_path.parent),
                timeout=timeout
            )
            
            # Log output
            if result.stdout:
                for line in result.stdout.split("\n"):
                    if line.strip() and "WARNING" not in line:
                        log_info(line.strip())
            
            return result
            
        except subprocess.TimeoutExpired:
            log_error(f"Command timed out after {timeout} seconds")
            return subprocess.CompletedProcess(cmd, 1, "", "Timeout")
        except Exception as e:
            log_error(f"Command failed: {e}")
            return subprocess.CompletedProcess(cmd, 1, "", str(e))
    
    def _verify_critical_signatures(self, build_dir: Path) -> bool:
        """Verify critical binaries are properly signed"""
        log_info("\n📝 Verifying critical binary signatures...")
        
        all_valid = True
        for name in CRITICAL_BINARIES:
            found = list(build_dir.rglob(name))
            if not found:
                log_warning(f"Critical file not found: {name}")
                continue
            
            for file_path in found:
                if not self._verify_signature(file_path):
                    all_valid = False
                    if name == "chrome_elf.dll":
                        log_error("🚨 CRITICAL: chrome_elf.dll is NOT properly signed!")
                        log_error("This WILL trigger Windows Defender!")
                        return False
        
        return all_valid
    
    def _verify_signature(self, file_path: Path) -> bool:
        """Verify a file's digital signature"""
        try:
            cmd = [
                "powershell",
                "-Command",
                f"(Get-AuthenticodeSignature '{file_path}').Status"
            ]
            
            result = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
            status = result.stdout.strip()
            
            if "Valid" in status:
                log_success(f"✅ {file_path.name}: Signature valid")
                return True
            else:
                log_error(f"❌ {file_path.name}: Invalid signature - {status}")
                return False
                
        except Exception as e:
            log_warning(f"Could not verify {file_path.name}: {e}")
            return False
    
    def _build_mini_installer(self, ctx: Context) -> bool:
        """Build the mini_installer.exe"""
        from ..compile import build_target
        return build_target(ctx, "mini_installer")
    
    def _validate_credentials(self, env: EnvConfig) -> None:
        """Validate SSL.com credentials"""
        missing = []
        
        if not env.code_sign_tool_path and not env.code_sign_tool_exe:
            missing.append("CODE_SIGN_TOOL_PATH or CODE_SIGN_TOOL_EXE")
        if not env.esigner_username:
            missing.append("ESIGNER_USERNAME")
        if not env.esigner_password:
            missing.append("ESIGNER_PASSWORD")
        if not env.esigner_totp_secret:
            missing.append("ESIGNER_TOTP_SECRET")
        
        if missing:
            raise ValidationError(f"Missing environment variables: {', '.join(missing)}")
    
    def _get_codesigntool_path(self, env: EnvConfig) -> Path:
        """Get path to CodeSignTool executable"""
        if env.code_sign_tool_exe:
            return Path(env.code_sign_tool_exe)
        elif env.code_sign_tool_path:
            return Path(env.code_sign_tool_path) / "CodeSignTool.bat"
        else:
            raise RuntimeError("CodeSignTool path not configured")
    
    def _check_time_sync(self) -> bool:
        """Check if system time is synchronized (required for TOTP)"""
        try:
            # Windows time sync check
            result = subprocess.run(
                ["w32tm", "/query", "/status"],
                capture_output=True,
                text=True,
                timeout=5
            )
            return result.returncode == 0
        except:
            return True  # Assume OK if check fails
    
    def _report_summary(self) -> None:
        """Report signing summary"""
        log_info("\n" + "="*60)
        log_info("Signing Summary:")
        log_info(f"  ✅ Successfully signed: {len(self.signed_files)}")
        log_info(f"  ❌ Failed: {len(self.failed_files)}")
        
        if self.failed_files:
            log_warning("\nFailed to sign:")
            for f in self.failed_files[:10]:
                log_warning(f"  • {f.name}")
            if len(self.failed_files) > 10:
                log_warning(f"  ... and {len(self.failed_files) - 10} more")


def check_signing_environment(env: Optional[EnvConfig] = None) -> bool:
    """Check if Windows signing environment is properly configured"""
    if env is None:
        env = EnvConfig()
    
    try:
        module = WindowsSignModule()
        module._validate_credentials(env)
        
        # Check CodeSignTool exists
        tool_path = module._get_codesigntool_path(env)
        if not tool_path.exists():
            log_error(f"CodeSignTool not found at: {tool_path}")
            return False
        
        # Check time sync
        if not module._check_time_sync():
            log_warning("System time may not be synchronized")
        
        log_success("✅ Signing environment properly configured")
        return True
        
    except Exception as e:
        log_error(f"Environment check failed: {e}")
        return False