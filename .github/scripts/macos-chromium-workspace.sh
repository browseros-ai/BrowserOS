#!/usr/bin/env bash
set -Eeuo pipefail

state_path=""
marker_name=".browseros-workspace-state.env"
workspace_parent_name="browseros-ci-apfs-workspaces"
workspace_prefix="browseros-ci-chromium-"
setup_cleanup_active=0

die() {
  echo "::error::$*" >&2
  exit 1
}

warn() {
  echo "::warning::$*" >&2
}

append_env() {
  local path="$1"
  local name="$2"
  local value="$3"
  if [ -n "$path" ]; then
    printf '%s=%s\n' "$name" "$value" >> "$path"
  fi
}

run_tag() {
  printf '%s-%s\n' "${GITHUB_RUN_ID:-local}" "${GITHUB_RUN_ATTEMPT:-1}"
}

default_state_path() {
  [ -n "${RUNNER_TEMP:-}" ] || return 1
  printf '%s/browseros-ci-chromium-workspace-%s.env\n' "$RUNNER_TEMP" "$(run_tag)"
}

resolve_state_path() {
  if [ -n "${MACOS_CHROMIUM_WORKSPACE_STATE_PATH:-}" ]; then
    state_path="$MACOS_CHROMIUM_WORKSPACE_STATE_PATH"
    return 0
  fi
  state_path="$(default_state_path)"
}

owned_state_path() {
  local expected
  expected="$(default_state_path 2>/dev/null)" || return 1
  [ "$1" = "$expected" ]
}

require_owned_state_path() {
  if ! owned_state_path "$state_path"; then
    die "Unexpected macOS Chromium workspace state path: $state_path"
  fi
}

expand_home() {
  local path="$1"
  printf '%s\n' "${path/#\~/$HOME}"
}

resolve_existing_dir() {
  local path
  path="$(expand_home "$1")"
  [ -d "$path" ] || return 1
  (cd "$path" && pwd -P)
}

stat_device() {
  stat -f '%d' "$1"
}

read_chromium_version() {
  local version_file="$1"
  awk -F= '
    $1 == "MAJOR" { major=$2 }
    $1 == "MINOR" { minor=$2 }
    $1 == "BUILD" { build=$2 }
    $1 == "PATCH" { patch=$2 }
    END {
      if (major == "" || minor == "" || build == "" || patch == "") {
        exit 1
      }
      printf "%s.%s.%s.%s\n", major, minor, build, patch
    }
  ' "$version_file"
}

default_version_file() {
  local script_dir repo_root
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
  repo_root="$(cd "$script_dir/../.." && pwd -P)"
  printf '%s/packages/browseros/CHROMIUM_VERSION\n' "$repo_root"
}

write_workspace_state() {
  local path="$1"
  {
    printf 'workspace_parent=%s\n' "$workspace_parent"
    printf 'workspace_root=%s\n' "$workspace_root"
    printf 'workspace_src=%s\n' "$workspace_src"
    printf 'base_root=%s\n' "$base_root"
    printf 'base_src=%s\n' "$base_src"
    printf 'base_head=%s\n' "$base_head"
    printf 'chromium_version=%s\n' "$chromium_version"
    printf 'run_tag=%s\n' "$tag"
  } > "$path"
}

read_state_file() {
  workspace_parent=""
  workspace_root=""
  workspace_src=""
  base_root=""
  base_src=""
  base_head=""
  chromium_version=""
  tag=""

  local state_line
  while IFS= read -r state_line; do
    case "$state_line" in
      workspace_parent=*) workspace_parent="${state_line#workspace_parent=}" ;;
      workspace_root=*) workspace_root="${state_line#workspace_root=}" ;;
      workspace_src=*) workspace_src="${state_line#workspace_src=}" ;;
      base_root=*) base_root="${state_line#base_root=}" ;;
      base_src=*) base_src="${state_line#base_src=}" ;;
      base_head=*) base_head="${state_line#base_head=}" ;;
      chromium_version=*) chromium_version="${state_line#chromium_version=}" ;;
      run_tag=*) tag="${state_line#run_tag=}" ;;
    esac
  done < "$1"
}

owned_workspace_path() {
  local path="$1"
  local parent="$2"

  [ -n "$path" ] || return 1
  [ -n "$parent" ] || return 1
  [ "$path" != "/" ] || return 1
  [ "$path" != "${HOME:-}" ] || return 1
  case "$path" in
    "$parent"/"$workspace_prefix"*) return 0 ;;
    *) return 1 ;;
  esac
}

marker_matches_workspace() {
  local marker="$1"
  local expected_workspace="$2"
  local expected_base="${3:-}"
  local marker_workspace=""
  local marker_base=""
  local line

  [ -f "$marker" ] || return 1
  while IFS= read -r line; do
    case "$line" in
      workspace_root=*) marker_workspace="${line#workspace_root=}" ;;
      base_root=*) marker_base="${line#base_root=}" ;;
    esac
  done < "$marker"

  [ "$marker_workspace" = "$expected_workspace" ] || return 1
  [ -z "$expected_base" ] || [ "$marker_base" = "$expected_base" ]
}

cleanup_workspace_path() {
  local target="$1"
  local parent="$2"
  local expected_base="${3:-}"
  local resolved_target resolved_parent marker

  [ -n "$target" ] || return 0
  [ -e "$target" ] || return 0
  [ -d "$target" ] || {
    warn "Ignoring non-directory Chromium workspace target: $target"
    return 0
  }

  resolved_target="$(resolve_existing_dir "$target")" || return 0
  resolved_parent="$(resolve_existing_dir "$parent")" || {
    warn "Ignoring Chromium workspace with missing parent: $parent"
    return 0
  }

  if ! owned_workspace_path "$resolved_target" "$resolved_parent"; then
    warn "Ignoring unexpected Chromium workspace path: $resolved_target"
    return 0
  fi

  marker="$resolved_target/$marker_name"
  if ! marker_matches_workspace "$marker" "$resolved_target" "$expected_base"; then
    warn "Ignoring Chromium workspace without matching owned marker: $resolved_target"
    return 0
  fi

  rm -rf "$resolved_target"
}

cleanup_workspace() {
  resolve_state_path || return 0
  if [ -z "$state_path" ] || [ ! -f "$state_path" ]; then
    return 0
  fi
  if ! owned_state_path "$state_path"; then
    warn "Ignoring unexpected macOS Chromium workspace state path: $state_path"
    return 0
  fi

  read_state_file "$state_path"
  cleanup_workspace_path "$workspace_root" "$workspace_parent" "$base_root"
  rm -f "$state_path"
}

reap_stale_workspaces() {
  local parent="$1"
  local current_tag="$2"
  local expected_base="$3"
  local candidate resolved marker marker_tag line

  [ -d "$parent" ] || return 0
  for candidate in "$parent"/"$workspace_prefix"*; do
    [ -e "$candidate" ] || continue
    [ -d "$candidate" ] || continue
    resolved="$(resolve_existing_dir "$candidate")" || continue
    [ "$(basename "$resolved")" != "$workspace_prefix$current_tag" ] || continue
    marker="$resolved/$marker_name"
    [ -f "$marker" ] || continue
    marker_tag=""
    while IFS= read -r line; do
      case "$line" in
        run_tag=*) marker_tag="${line#run_tag=}" ;;
      esac
    done < "$marker"
    [ "$marker_tag" != "$current_tag" ] || continue
    cleanup_workspace_path "$resolved" "$parent" "$expected_base"
  done
}

check_no_browseros_outputs() {
  local out_dir="$1/out"
  local found
  [ -d "$out_dir" ] || return 0
  found="$(
    find "$out_dir" -maxdepth 1 -type d \
      \( -name 'Default_browseros_*' -o -name 'Default_browserclaw_*' \) \
      -print -quit
  )"
  [ -z "$found" ] || die "Could not remove BrowserOS output state from the CI-owned Chromium base: $found"
}

check_git_repo_clean() {
  local repo="$1"
  local status

  status="$(git -C "$repo" status --porcelain=v1 --untracked-files=all)"
  [ -z "$status" ] || die "Could not restore the CI-owned Chromium base repository to a clean state: $repo"
}

repair_git_repo() {
  local repo="$1"
  local ref="$2"

  git -C "$repo" reset --hard "$ref"
  git -C "$repo" clean -fd
  check_git_repo_clean "$repo"
}

repair_nested_git_repos() {
  local git_meta repo

  while IFS= read -r git_meta; do
    repo="$(resolve_existing_dir "$(dirname "$git_meta")")" || continue
    [ "$repo" != "$base_src" ] || continue
    repair_git_repo "$repo" HEAD
  done < <(
    find "$base_root" \
      \( -path "$base_src/out" -o -path "$base_src/out/*" \) -prune -o \
      -name .git -print -prune
  )
}

remove_browseros_outputs() {
  local out_dir="$1/out"
  local output

  [ -d "$out_dir" ] || return 0
  for output in \
    "$out_dir"/Default_browseros_* \
    "$out_dir"/Default_browserclaw_*; do
    [ -d "$output" ] || continue
    rm -rf "$output"
  done
  check_no_browseros_outputs "$1"
}

verify_cow_clone_support() {
  local parent="$1"
  local probe_dir="$parent/.browseros-ci-apfs-probe-$(run_tag)-$$"
  local source_file="$probe_dir/source"
  local clone_file="$probe_dir/clone"

  rm -rf "$probe_dir"
  mkdir -p "$probe_dir"
  printf 'browseros apfs clone probe\n' > "$source_file"
  if ! cp -c "$source_file" "$clone_file"; then
    rm -rf "$probe_dir"
    die "APFS copy-on-write clone probe failed under $parent; place the workspace parent on APFS with clonefile support"
  fi
  if ! cmp -s "$source_file" "$clone_file"; then
    rm -rf "$probe_dir"
    die "APFS copy-on-write clone probe produced different bytes"
  fi
  rm -rf "$probe_dir"
}

prepare_base() {
  local version_file="$1"
  local current_head pin_head

  [ -f "$base_root/.gclient" ] || die "Chromium base root is missing .gclient: $base_root"
  [ -e "$base_src/.git" ] || die "Chromium base src is missing .git: $base_src"
  [ -f "$version_file" ] || die "Chromium version file not found: $version_file"

  chromium_version="$(read_chromium_version "$version_file")" \
    || die "Could not parse Chromium version file: $version_file"
  current_head="$(git -C "$base_src" rev-parse HEAD)"
  pin_head="$(git -C "$base_src" rev-parse "refs/tags/$chromium_version^{commit}")" \
    || die "Chromium base is missing pinned tag $chromium_version; refresh the warm base checkout before rerunning"
  [ "$current_head" = "$pin_head" ] \
    || die "Chromium base HEAD $current_head does not match pinned $chromium_version ($pin_head); refresh the warm base checkout before rerunning"

  # BROWSEROS_CHROMIUM_SRC is infrastructure-owned clone input, never a
  # developer workspace. Resetting it is intentionally destructive: all build
  # mutations belong in the disposable APFS clone created after this seam.
  repair_git_repo "$base_src" "$pin_head"
  repair_nested_git_repos
  remove_browseros_outputs "$base_src"
  base_head="$(git -C "$base_src" rev-parse HEAD)"
}

cleanup_after_setup_error() {
  local status="$?"
  [ "$setup_cleanup_active" = "1" ] || return 0
  setup_cleanup_active=0
  if [ -n "${workspace_root:-}" ] && [ -n "${workspace_parent:-}" ]; then
    cleanup_workspace_path "$workspace_root" "$workspace_parent" "${base_root:-}" || true
  fi
  if owned_state_path "$state_path"; then
    rm -f "$state_path"
  fi
  exit "$status"
}

setup_workspace() {
  local requested_base_src="${1:-${CHROMIUM_SRC:-}}"
  local version_file="${BROWSEROS_CHROMIUM_VERSION_FILE:-}"
  local base_parent base_device parent_device

  [ -n "$requested_base_src" ] || die "usage: $0 setup <persistent-chromium-src>"
  [ "$(uname -s)" = "Darwin" ] || die "Disposable APFS Chromium workspaces require macOS"
  [ -n "${RUNNER_TEMP:-}" ] || die "RUNNER_TEMP is required"
  resolve_state_path
  require_owned_state_path

  cleanup_workspace

  base_src="$(resolve_existing_dir "$requested_base_src")" \
    || die "Persistent Chromium src does not exist: $requested_base_src"
  [ "$(basename "$base_src")" = "src" ] || die "Persistent Chromium path must point to src: $base_src"
  base_root="$(resolve_existing_dir "$base_src/..")" \
    || die "Could not resolve Chromium gclient root from $base_src"
  [ -f "$base_root/.gclient" ] || die "Chromium base root is missing .gclient: $base_root"

  base_parent="$(resolve_existing_dir "$base_root/..")"
  workspace_parent="$base_parent/$workspace_parent_name"
  mkdir -p "$workspace_parent"
  workspace_parent="$(resolve_existing_dir "$workspace_parent")"
  base_device="$(stat_device "$base_root")"
  parent_device="$(stat_device "$workspace_parent")"
  [ "$base_device" = "$parent_device" ] \
    || die "Chromium workspace parent is not on the base checkout volume: $workspace_parent; move the base or workspace parent onto one APFS volume"

  tag="$(run_tag)"
  reap_stale_workspaces "$workspace_parent" "$tag" "$base_root"
  version_file="${version_file:-$(default_version_file)}"
  prepare_base "$version_file"

  workspace_root="$workspace_parent/$workspace_prefix$tag"
  workspace_src="$workspace_root/src"
  if [ -e "$workspace_root" ]; then
    cleanup_workspace_path "$workspace_root" "$workspace_parent" "$base_root"
  fi
  [ ! -e "$workspace_root" ] || die "Owned Chromium workspace already exists and could not be cleaned: $workspace_root"

  mkdir "$workspace_root"
  write_workspace_state "$workspace_root/$marker_name"
  setup_cleanup_active=1
  trap cleanup_after_setup_error EXIT

  verify_cow_clone_support "$workspace_parent"
  cp -cR "$base_root"/. "$workspace_root"
  write_workspace_state "$workspace_root/$marker_name"
  [ -d "$workspace_src" ] || die "APFS clone did not produce src: $workspace_src"
  [ "$(git -C "$workspace_src" rev-parse HEAD)" = "$base_head" ] \
    || die "APFS clone source HEAD does not match base HEAD"

  write_workspace_state "$state_path"
  append_env "${GITHUB_ENV:-}" CHROMIUM_SRC "$workspace_src"
  append_env "${GITHUB_ENV:-}" MACOS_CHROMIUM_WORKSPACE_STATE_PATH "$state_path"
  append_env "${GITHUB_OUTPUT:-}" chromium_src "$workspace_src"
  append_env "${GITHUB_OUTPUT:-}" workspace_root "$workspace_root"
  append_env "${GITHUB_OUTPUT:-}" base_src "$base_src"
  append_env "${GITHUB_OUTPUT:-}" base_head "$base_head"
  append_env "${GITHUB_OUTPUT:-}" state_path "$state_path"
  setup_cleanup_active=0
  trap - EXIT
}

case "${1:-}" in
  setup)
    shift
    setup_workspace "$@"
    ;;
  cleanup)
    cleanup_workspace
    ;;
  *)
    echo "usage: $0 setup <persistent-chromium-src>|cleanup" >&2
    exit 2
    ;;
esac
