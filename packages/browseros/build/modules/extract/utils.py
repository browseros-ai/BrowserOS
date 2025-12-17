"""
Shared utilities for Dev CLI operations

This module provides robust utilities for git operations, diff parsing,
and patch management with comprehensive error handling.
"""

import subprocess
import click
import re
from pathlib import Path
from typing import Optional, List, Dict, Tuple
from enum import Enum
from dataclasses import dataclass
from ...common.context import Context
from ...common.utils import log_error, log_success, log_warning


class FileOperation(Enum):
    """Types of file operations in a diff"""

    ADD = "add"
    MODIFY = "modify"
    DELETE = "delete"
    RENAME = "rename"
    COPY = "copy"
    BINARY = "binary"


@dataclass
class FilePatch:
    """Represents a single file's patch information"""

    file_path: str
    operation: FileOperation
    old_path: Optional[str] = None  # For renames/copies
    patch_content: Optional[str] = None
    is_binary: bool = False
    similarity: Optional[int] = None  # For renames (percentage)


class GitError(Exception):
    """Custom exception for git operations"""

    pass


def run_git_command(
    cmd: List[str],
    cwd: Path,
    capture: bool = True,
    check: bool = False,
    timeout: Optional[int] = None,
    binary_output: bool = False,
) -> subprocess.CompletedProcess:
    """Run a git command and return the result

    Args:
        cmd: Command to run
        cwd: Working directory
        capture: Whether to capture output
        check: Whether to raise on non-zero return
        timeout: Command timeout in seconds
        binary_output: If True, handle binary output (don't decode as text)

    Returns:
        CompletedProcess result

    Raises:
        GitError: If command fails and check=True
    """
    try:
        # For commands that might output binary data (like git diff with binary files),
        # we need to handle them specially
        if binary_output or ("diff" in cmd and "--binary" not in cmd):
            # First try with text mode
            try:
                result = subprocess.run(
                    cmd,
                    cwd=cwd,
                    capture_output=capture,
                    text=True,
                    check=False,
                    timeout=timeout or 60,
                    errors="replace",  # Replace invalid UTF-8 sequences
                )
            except UnicodeDecodeError:
                # Fall back to binary mode
                result = subprocess.run(
                    cmd,
                    cwd=cwd,
                    capture_output=capture,
                    text=False,
                    check=False,
                    timeout=timeout or 60,
                )
                # Convert to text with error handling
                if result.stdout:
                    result.stdout = result.stdout.decode("utf-8", errors="replace")
                if result.stderr:
                    result.stderr = result.stderr.decode("utf-8", errors="replace")
        else:
            result = subprocess.run(
                cmd,
                cwd=cwd,
                capture_output=capture,
                text=True,
                check=False,
                timeout=timeout or 60,
            )

        if check and result.returncode != 0:
            error_msg = result.stderr or result.stdout or "Unknown error"
            raise GitError(f"Git command failed: {' '.join(cmd)}\nError: {error_msg}")

        return result
    except subprocess.TimeoutExpired:
        log_error(f"Git command timed out after {timeout} seconds: {' '.join(cmd)}")
        raise GitError(f"Command timed out: {' '.join(cmd)}")
    except Exception as e:
        log_error(f"Failed to run git command: {' '.join(cmd)}")
        raise GitError(f"Command failed: {e}")


def validate_git_repository(path: Path) -> bool:
    """Validate that a path is a git repository"""
    try:
        result = run_git_command(
            ["git", "rev-parse", "--git-dir"], cwd=path, check=False
        )
        return result.returncode == 0
    except GitError:
        return False


def validate_commit_exists(commit_hash: str, chromium_src: Path) -> bool:
    """Validate that a commit exists in the repository"""
    try:
        result = run_git_command(
            ["git", "rev-parse", "--verify", f"{commit_hash}^{{commit}}"],
            cwd=chromium_src,
        )

        if result.returncode != 0:
            log_error(f"Commit '{commit_hash}' not found in repository")
            return False
        return True
    except GitError as e:
        log_error(f"Failed to validate commit: {e}")
        return False


def get_commit_changed_files(commit_hash: str, chromium_src: Path) -> List[str]:
    """Get list of files changed in a commit"""
    try:
        result = run_git_command(
            ["git", "diff-tree", "--no-commit-id", "--name-only", "-r", commit_hash],
            cwd=chromium_src,
        )

        if result.returncode != 0:
            log_error(f"Failed to get changed files for commit {commit_hash}")
            return []

        files = [f.strip() for f in result.stdout.strip().split("\n") if f.strip()]
        return files
    except GitError as e:
        log_error(f"Error getting changed files: {e}")
        return []


def parse_diff_output(diff_output: str) -> Dict[str, FilePatch]:
    """
    Parse git diff output into individual file patches with full metadata.

    Handles:
    - Regular file modifications
    - New files
    - Deleted files
    - Binary files
    - File renames
    - File copies
    - Mode changes

    Returns:
        Dict mapping file path to FilePatch objects
    """
    patches = {}
    current_file = None
    current_patch_lines = []
    current_operation = FileOperation.MODIFY
    is_binary = False
    old_path = None
    similarity = None

    lines = diff_output.splitlines()
    i = 0

    while i < len(lines):
        line = lines[i]

        # Start of a new file diff
        if line.startswith("diff --git"):
            # Save previous patch if exists
            if current_file and current_patch_lines:
                patch_content = (
                    "\n".join(current_patch_lines) if not is_binary else None
                )
                patches[current_file] = FilePatch(
                    file_path=current_file,
                    operation=current_operation,
                    old_path=old_path,
                    patch_content=patch_content,
                    is_binary=is_binary,
                    similarity=similarity,
                )

            # Parse file paths from diff line
            match = re.match(r"diff --git a/(.*) b/(.*)", line)
            if match:
                _old_file = match.group(1)
                new_file = match.group(2)
                current_file = new_file
                current_patch_lines = [line]
                current_operation = FileOperation.MODIFY
                is_binary = False
                old_path = None
                similarity = None
            else:
                log_warning(f"Could not parse diff line: {line}")
                current_file = None
                current_patch_lines = []

            i += 1
            continue

        # Check for file metadata
        if current_file:
            if line.startswith("deleted file"):
                current_operation = FileOperation.DELETE
                current_patch_lines.append(line)
            elif line.startswith("new file"):
                current_operation = FileOperation.ADD
                current_patch_lines.append(line)
            elif line.startswith("similarity index"):
                # Extract similarity percentage for renames
                match = re.match(r"similarity index (\d+)%", line)
                if match:
                    similarity = int(match.group(1))
                current_patch_lines.append(line)
            elif line.startswith("rename from"):
                current_operation = FileOperation.RENAME
                old_path = line[12:].strip()  # Remove 'rename from '
                current_patch_lines.append(line)
            elif line.startswith("rename to"):
                # Confirm rename operation
                current_patch_lines.append(line)
            elif line.startswith("copy from"):
                current_operation = FileOperation.COPY
                old_path = line[10:].strip()  # Remove 'copy from '
                current_patch_lines.append(line)
            elif line.startswith("copy to"):
                # Confirm copy operation
                current_patch_lines.append(line)
            elif line == "Binary files differ" or line.startswith("Binary files"):
                is_binary = True
                current_operation = (
                    FileOperation.BINARY
                    if current_operation == FileOperation.MODIFY
                    else current_operation
                )
                current_patch_lines.append(line)
            elif (
                line.startswith("index ")
                or line.startswith("---")
                or line.startswith("+++")
            ):
                current_patch_lines.append(line)
            elif line.startswith("@@"):
                # Hunk header
                current_patch_lines.append(line)
            elif line.startswith("+") or line.startswith("-") or line.startswith(" "):
                # Actual diff content
                current_patch_lines.append(line)
            elif line.startswith("\\"):
                # Special markers like "\ No newline at end of file"
                current_patch_lines.append(line)
            else:
                # Other content
                current_patch_lines.append(line)

        i += 1

    # Save last patch
    if current_file and current_patch_lines:
        patch_content = "\n".join(current_patch_lines) if not is_binary else None
        patches[current_file] = FilePatch(
            file_path=current_file,
            operation=current_operation,
            old_path=old_path,
            patch_content=patch_content,
            is_binary=is_binary,
            similarity=similarity,
        )

    return patches


def write_patch_file(ctx: Context, file_path: str, patch_content: str) -> bool:
    """
    Write a patch file to chromium_src directory structure.

    Args:
        ctx: Build context
        file_path: Path of the file being patched
        patch_content: The patch content to write

    Returns:
        True if successful, False otherwise
    """
    # Construct output path
    output_path = ctx.get_patch_path_for_file(file_path)

    # Create directory structure
    output_path.parent.mkdir(parents=True, exist_ok=True)

    try:
        # Ensure patch ends with newline
        if patch_content and not patch_content.endswith("\n"):
            patch_content += "\n"

        output_path.write_text(patch_content, encoding="utf-8")
        log_success(f"  Written: {output_path.relative_to(ctx.root_dir)}")
        return True
    except Exception as e:
        log_error(f"  Failed to write {output_path}: {e}")
        return False


def create_deletion_marker(ctx: Context, file_path: str) -> Optional[bool]:
    """
    Create a marker file for deleted files.

    If existing patch files exist for this file, prompts user to confirm
    their removal and choose the appropriate action.

    Args:
        ctx: Build context
        file_path: Path of the deleted file

    Returns:
        True if marker created successfully (or patch removed without marker)
        False if failed
        None if user chose to skip
    """
    patches_dir = ctx.get_patches_dir()
    base_path = patches_dir / file_path

    # Check for existing patch-related files that would conflict
    existing_files = []

    # Check if raw patch file exists (no suffix - this is how patches are stored)
    if base_path.exists():
        existing_files.append(base_path)

    # Also check for marker files
    for suffix in [".patch", ".binary", ".rename"]:
        check_path = base_path.with_suffix(base_path.suffix + suffix)
        if check_path.exists():
            existing_files.append(check_path)

    if existing_files:
        log_warning(
            f"File '{file_path}' is being deleted, but existing patch(es) found:"
        )
        for ef in existing_files:
            log_warning(f"  - {ef.relative_to(ctx.root_dir)}")

        click.echo("\nHow should this be handled?")
        click.echo("  1) Remove patch and create .deleted marker (file exists in upstream)")
        click.echo("  2) Remove patch only (file was added by your patches, not in upstream)")
        click.echo("  3) Skip (keep existing patch, don't record deletion)")

        choice = click.prompt("Choice", type=click.Choice(["1", "2", "3"]), default="1")

        if choice == "3":
            log_warning(f"  Skipped: {file_path}")
            return None

        # Remove existing files for choices 1 and 2
        for ef in existing_files:
            try:
                ef.unlink()
                log_warning(f"  Removed: {ef.relative_to(ctx.root_dir)}")
            except Exception as e:
                log_error(f"  Failed to remove {ef}: {e}")
                return False

        if choice == "2":
            # User chose to remove patch only, no .deleted marker
            log_success(f"  Removed patch for: {file_path} (no .deleted marker)")
            return True

    # Create deletion marker
    marker_path = base_path.with_suffix(base_path.suffix + ".deleted")
    marker_path.parent.mkdir(parents=True, exist_ok=True)

    try:
        marker_content = f"File deleted in patch\nOriginal path: {file_path}\n"
        marker_path.write_text(marker_content, encoding="utf-8")
        log_warning(f"  Marked deleted: {marker_path.relative_to(ctx.root_dir)}")
        return True
    except Exception as e:
        log_error(f"  Failed to create deletion marker: {e}")
        return False


def create_binary_marker(
    ctx: Context, file_path: str, operation: FileOperation
) -> bool:
    """
    Create a marker file for binary files.

    Args:
        ctx: Build context
        file_path: Path of the binary file
        operation: The operation type

    Returns:
        True if successful, False otherwise
    """
    marker_path = ctx.get_patches_dir() / file_path
    marker_path = marker_path.with_suffix(marker_path.suffix + ".binary")

    marker_path.parent.mkdir(parents=True, exist_ok=True)

    try:
        marker_content = (
            f"Binary file\nOperation: {operation.value}\nOriginal path: {file_path}\n"
        )
        marker_path.write_text(marker_content, encoding="utf-8")
        log_warning(f"  Binary file marked: {marker_path.relative_to(ctx.root_dir)}")
        return True
    except Exception as e:
        log_error(f"  Failed to create binary marker: {e}")
        return False


def apply_single_patch(
    patch_path: Path, chromium_src: Path, interactive: bool = True
) -> Tuple[bool, str]:
    """
    Apply a single patch file to chromium source with multiple strategies.

    Tries in order:
    1. Standard git apply
    2. Three-way merge
    3. Patch command fallback
    4. Interactive conflict resolution

    Returns:
        Tuple of (success, message)
    """
    if not patch_path.exists():
        return False, f"Patch file not found: {patch_path}"

    # Check if it's a deletion marker
    if patch_path.suffix == ".deleted":
        # Handle file deletion
        file_path = patch_path.stem
        target_file = chromium_src / file_path
        if target_file.exists():
            try:
                target_file.unlink()
                return True, f"Deleted: {file_path}"
            except Exception as e:
                return False, f"Failed to delete {file_path}: {e}"
        else:
            return True, f"Already deleted: {file_path}"

    # Check if it's a binary marker
    if patch_path.suffix == ".binary":
        return False, f"Binary file patch not supported: {patch_path.name}"

    # Try standard apply
    result = run_git_command(["git", "apply", "-p1", str(patch_path)], cwd=chromium_src)

    if result.returncode == 0:
        return True, f"Applied: {patch_path.name}"

    # Try 3-way merge
    result = run_git_command(
        ["git", "apply", "-p1", "--3way", str(patch_path)], cwd=chromium_src
    )

    if result.returncode == 0:
        return True, f"Applied (3-way): {patch_path.name}"

    # Try with whitespace options
    result = run_git_command(
        ["git", "apply", "-p1", "--whitespace=fix", str(patch_path)], cwd=chromium_src
    )

    if result.returncode == 0:
        return True, f"Applied (whitespace fixed): {patch_path.name}"

    # Handle conflict
    if interactive:
        return handle_patch_conflict(patch_path, chromium_src, result.stderr)
    else:
        return False, f"Failed: {patch_path.name} - {result.stderr}"


def handle_patch_conflict(
    patch_path: Path, chromium_src: Path, error_msg: str = ""
) -> Tuple[bool, str]:
    """Handle patch conflict interactively with detailed options"""
    click.echo(f"\n{click.style('CONFLICT:', fg='red', bold=True)} {patch_path}")

    if error_msg:
        # Parse error message for more context
        lines = error_msg.strip().split("\n")
        for line in lines[:5]:  # Show first 5 lines of error
            click.echo(f"  {line}")

    click.echo("\nOptions:")
    click.echo("  1) Fix manually and continue")
    click.echo("  2) Skip this patch")
    click.echo("  3) Try with reduced context (--unidiff-zero)")
    click.echo("  4) Show patch content")
    click.echo("  5) Abort all remaining patches")

    while True:
        choice = click.prompt("Enter choice (1-5)", type=str)

        if choice == "1":
            click.prompt("Fix the conflicts manually and press Enter to continue")
            return True, f"Manually fixed: {patch_path.name}"
        elif choice == "2":
            return True, f"Skipped: {patch_path.name}"
        elif choice == "3":
            # Try with reduced context
            result = run_git_command(
                ["git", "apply", "-p1", "--unidiff-zero", str(patch_path)],
                cwd=chromium_src,
            )
            if result.returncode == 0:
                return True, f"Applied (reduced context): {patch_path.name}"
            else:
                click.echo("Failed with reduced context too")
                continue
        elif choice == "4":
            # Show patch content
            try:
                content = patch_path.read_text()
                lines = content.split("\n")
                # Show first 50 lines
                click.echo("\n--- Patch Content (first 50 lines) ---")
                for line in lines[:50]:
                    click.echo(line)
                if len(lines) > 50:
                    click.echo(f"... and {len(lines) - 50} more lines")
                click.echo("--- End of Preview ---\n")
            except Exception as e:
                click.echo(f"Failed to read patch: {e}")
            continue
        elif choice == "5":
            return False, "Aborted by user"
        else:
            click.echo("Invalid choice. Please enter 1-5.")


def create_git_commit(chromium_src: Path, message: str) -> bool:
    """Create a git commit with the given message"""
    # Check if there are changes to commit
    result = run_git_command(["git", "status", "--porcelain"], cwd=chromium_src)

    if not result.stdout.strip():
        log_warning("Nothing to commit, working tree clean")
        return True

    # Stage all changes
    result = run_git_command(["git", "add", "-A"], cwd=chromium_src)

    if result.returncode != 0:
        log_error("Failed to stage changes")
        return False

    # Create commit
    result = run_git_command(["git", "commit", "-m", message], cwd=chromium_src)

    if result.returncode != 0:
        if "nothing to commit" in result.stdout:
            log_warning("Nothing to commit")
        else:
            log_error(f"Failed to create commit: {result.stderr}")
        return False

    log_success(f"Created commit: {message}")
    return True


def get_commit_info(commit_hash: str, chromium_src: Path) -> Optional[Dict[str, str]]:
    """Get detailed information about a commit"""
    try:
        # Get commit info in a structured format
        result = run_git_command(
            [
                "git",
                "show",
                "--format=%H%n%an%n%ae%n%at%n%s%n%b",
                "--no-patch",
                commit_hash,
            ],
            cwd=chromium_src,
        )

        if result.returncode != 0:
            return None

        lines = result.stdout.strip().split("\n")
        if len(lines) >= 5:
            return {
                "hash": lines[0],
                "author_name": lines[1],
                "author_email": lines[2],
                "timestamp": lines[3],
                "subject": lines[4],
                "body": "\n".join(lines[5:]) if len(lines) > 5 else "",
            }
        return None
    except GitError:
        return None


def prompt_yes_no(question: str, default: bool = False) -> bool:
    """Prompt user for yes/no question"""
    default_str = "Y/n" if default else "y/N"
    result = click.prompt(
        f"{question} [{default_str}]", type=str, default="y" if default else "n"
    )
    return result.lower() in ("y", "yes")


def log_extraction_summary(file_patches: Dict[str, FilePatch]):
    """Log a detailed summary of extracted patches"""
    total = len(file_patches)

    # Count by operation type
    operations = {op: 0 for op in FileOperation}
    binary_count = 0

    for patch in file_patches.values():
        operations[patch.operation] += 1
        if patch.is_binary:
            binary_count += 1

    click.echo("\n" + click.style("Extraction Summary", fg="green", bold=True))
    click.echo("=" * 60)
    click.echo(f"Total files:     {total}")
    click.echo("-" * 40)

    if operations[FileOperation.ADD] > 0:
        click.echo(f"New files:       {operations[FileOperation.ADD]}")
    if operations[FileOperation.MODIFY] > 0:
        click.echo(f"Modified:        {operations[FileOperation.MODIFY]}")
    if operations[FileOperation.DELETE] > 0:
        click.echo(f"Deleted:         {operations[FileOperation.DELETE]}")
    if operations[FileOperation.RENAME] > 0:
        click.echo(f"Renamed:         {operations[FileOperation.RENAME]}")
    if operations[FileOperation.COPY] > 0:
        click.echo(f"Copied:          {operations[FileOperation.COPY]}")
    if binary_count > 0:
        click.echo(f"Binary files:    {binary_count}")

    click.echo("=" * 60)


def log_apply_summary(results: List[Tuple[str, bool, str]]):
    """Log a detailed summary of applied patches"""
    total = len(results)
    successful = sum(1 for _, success, _ in results if success)
    failed = total - successful

    click.echo(
        "\n"
        + click.style(
            "Apply Summary", fg="green" if failed == 0 else "yellow", bold=True
        )
    )
    click.echo("=" * 60)
    click.echo(f"Total patches:   {total}")
    click.echo(f"Successful:      {successful}")
    click.echo(f"Failed:          {failed}")
    click.echo("=" * 60)

    if failed > 0:
        click.echo("\n" + click.style("Failed patches:", fg="red", bold=True))
        for file_path, success, message in results:
            if not success:
                click.echo(f"  ✗ {file_path}: {message}")
