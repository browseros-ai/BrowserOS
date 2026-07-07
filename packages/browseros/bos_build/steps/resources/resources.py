#!/usr/bin/env python3
"""Resource management module for BrowserOS build system"""

import glob
import shutil
import yaml
import subprocess
from pathlib import Path
from ...core.step import Step, ValidationError, step
from ...core.context import Context
from ...lib.utils import log_info, log_success, log_error, log_warning, get_platform


@step("resources", phase="prep")
class ResourcesModule(Step):
    produces = []
    requires = []
    description = "Copy resources (icons, extensions) to Chromium"

    def validate(self, ctx: Context) -> None:
        copy_config_path = ctx.get_copy_resources_config()
        if not copy_config_path.exists():
            raise ValidationError(f"Copy configuration file not found: {copy_config_path}")

    def execute(self, ctx: Context) -> None:
        log_info("\n📦 Copying resources...")
        if not copy_resources_impl(ctx, commit_each=False):
            raise RuntimeError("Failed to copy resources")


def copy_resources_impl(ctx: Context, commit_each: bool = False) -> bool:
    """Copy configured resources into the Chromium checkout."""
    log_info("\n📦 Copying resources...")

    copy_config_path = ctx.get_copy_resources_config()
    if not copy_config_path.exists():
        log_error(f"Copy configuration file not found: {copy_config_path}")
        raise FileNotFoundError(
            f"Copy configuration file not found: {copy_config_path}"
        )

    with open(copy_config_path, "r") as f:
        config = yaml.safe_load(f)

    if "copy_operations" not in config:
        log_info("⚠️  No copy_operations defined in configuration")
        return True

    if commit_each:
        log_info(
            "📝 Git commit mode enabled - will create a commit after each resource copy"
        )

    all_ok = True

    for operation in config["copy_operations"]:
        name = operation.get("name", "Unnamed operation")
        source = operation["source"]
        destination = operation["destination"]
        op_type = operation.get("type", "directory")
        build_type_condition = operation.get("build_type")
        os_condition = operation.get("os")
        arch_condition = operation.get("arch")
        product_condition = operation.get("product")

        if not product_matches(product_condition, ctx.product.id):
            log_info(
                f"  ⏭️  Skipping {name} (product: {product_condition}, current: {ctx.product.id})"
            )
            continue

        clear_destination = operation.get("clear_destination", False)
        renames = operation.get("renames")

        if build_type_condition and build_type_condition != ctx.build_type:
            log_info(
                f"  ⏭️  Skipping {name} (build_type: {build_type_condition}, current: {ctx.build_type})"
            )
            continue

        if os_condition:
            current_os = get_platform()
            if current_os not in os_condition:
                log_info(
                    f"  ⏭️  Skipping {name} (os: {os_condition}, current: {current_os})"
                )
                continue

        if arch_condition:
            if ctx.architecture not in arch_condition:
                log_info(
                    f"  ⏭️  Skipping {name} (arch: {arch_condition}, current: {ctx.architecture})"
                )
                continue

        src_path = ctx.root_dir / source
        dst_base = ctx.chromium_src / destination

        log_info(f"  • {name}")

        try:
            copied = False
            if clear_destination:
                clear_path(dst_base)
            if op_type == "directory":
                if src_path.exists() and src_path.is_dir():
                    dst_path = dst_base
                    dst_path.mkdir(parents=True, exist_ok=True)
                    shutil.copytree(src_path, dst_path, dirs_exist_ok=True)
                    copied = True
                    log_info(f"    ✓ Copied directory: {source} → {destination}")
                    if commit_each:
                        commit_resource_copy(
                            name, source, destination, ctx.chromium_src
                        )
                else:
                    log_warning(f"    Source directory not found: {source}")

            elif op_type == "files":
                files = glob.glob(str(ctx.root_dir / source))
                if files:
                    dst_base.mkdir(parents=True, exist_ok=True)
                    for file_path in files:
                        file_path = Path(file_path)
                        if file_path.is_file():
                            shutil.copy2(file_path, dst_base)
                            copied = True
                    log_info(
                        f"    ✓ Copied {len(files)} files: {source} → {destination}"
                    )
                    if commit_each:
                        commit_resource_copy(
                            name, source, destination, ctx.chromium_src
                        )
                else:
                    log_warning(f"    No files found matching: {source}")

            elif op_type == "file":
                if src_path.exists() and src_path.is_file():
                    dst_base.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(src_path, dst_base)
                    copied = True
                    log_info(f"    ✓ Copied file: {source} → {destination}")
                    if commit_each:
                        commit_resource_copy(
                            name, source, destination, ctx.chromium_src
                        )
                else:
                    log_warning(f"    Source file not found: {source}")

            if copied and renames:
                apply_renames(dst_base, renames)

        except Exception as e:
            log_error(f"    Error: {e}")
            all_ok = False

    if all_ok:
        log_success("Resources copied")
    return all_ok


def product_matches(product_condition, product_id: str) -> bool:
    """Return whether a config operation applies to the active product."""
    if product_condition is None:
        return True
    if product_condition == "all":
        raise ValueError("Use a missing product field for all products, not product: all")
    products = (
        [product_condition] if isinstance(product_condition, str) else product_condition
    )
    return product_id in products


def apply_renames(base: Path, renames) -> None:
    """Rename declared relative paths under an already-copied destination."""
    if not isinstance(renames, list):
        raise ValueError("renames must be a list")

    for rename in renames:
        if not isinstance(rename, dict):
            raise ValueError("rename entries must be mappings")
        src_rel = _safe_relative_path(rename.get("from"), "from")
        dst_rel = _safe_relative_path(rename.get("to"), "to")
        src = base / src_rel
        dst = base / dst_rel
        if not src.is_file():
            raise FileNotFoundError(f"rename source not found: {src_rel.as_posix()}")
        dst.parent.mkdir(parents=True, exist_ok=True)
        if dst.exists():
            if dst.is_dir():
                raise IsADirectoryError(f"rename target is a directory: {dst_rel}")
            dst.unlink()
        src.rename(dst)
        log_info(f"    ✓ Renamed {src_rel.as_posix()} → {dst_rel.as_posix()}")


def clear_path(path: Path) -> None:
    """Remove an existing destination before a mutually exclusive copy."""
    if not path.exists():
        return
    if path.is_dir():
        shutil.rmtree(path)
        return
    path.unlink()


def _safe_relative_path(raw_path, field: str) -> Path:
    if not isinstance(raw_path, str) or not raw_path:
        raise ValueError(f"rename {field} must be a non-empty relative path")
    rel = Path(raw_path)
    if rel.is_absolute() or ".." in rel.parts or rel == Path("."):
        raise ValueError(f"rename {field} is unsafe: {raw_path}")
    return rel


def commit_resource_copy(
    name: str, source: str, destination: str, chromium_src: Path
) -> bool:
    """Create a git commit for the copied resource"""
    try:
        # Stage all changes
        cmd_add = ["git", "add", "-A"]
        result = subprocess.run(
            cmd_add, capture_output=True, text=True, cwd=chromium_src
        )
        if result.returncode != 0:
            log_warning(f"Failed to stage changes for resource copy: {name}")
            if result.stderr:
                log_warning(f"Error: {result.stderr}")
            return False

        # Create commit message
        commit_message = f"resource: {name.lower()}"

        # Create the commit
        cmd_commit = ["git", "commit", "-m", commit_message]
        result = subprocess.run(
            cmd_commit, capture_output=True, text=True, cwd=chromium_src
        )

        if result.returncode == 0:
            log_success(f"📝 Created commit for resource: {name}")
            return True
        else:
            log_warning(f"Failed to commit resource copy: {name}")
            if result.stderr:
                log_warning(f"Error: {result.stderr}")
            return False

    except Exception as e:
        log_warning(f"Error creating commit for resource {name}: {e}")
        return False
