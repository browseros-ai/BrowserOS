#!/usr/bin/env bash
set -euo pipefail

state_path="${MACOS_SIGNING_STATE_PATH:-${RUNNER_TEMP:-}/browseros-ci-signing-keychain-state.env}"

owned_state_path() {
  [ -n "${RUNNER_TEMP:-}" ] \
    && [ "$1" = "$RUNNER_TEMP/browseros-ci-signing-keychain-state.env" ]
}

require_owned_state_path() {
  if ! owned_state_path "$state_path"; then
    echo "::error::Unexpected macOS signing state path: $state_path"
    exit 1
  fi
}

owned_keychain_path() {
  case "$1" in
    "$RUNNER_TEMP"/browseros-ci-signing-*.keychain-db) return 0 ;;
    *) return 1 ;;
  esac
}

owned_cert_path() {
  case "$1" in
    "$RUNNER_TEMP"/browseros-signing-cert-*.p12) return 0 ;;
    *) return 1 ;;
  esac
}

owned_profile_path() {
  case "$1" in
    "$RUNNER_TEMP"/browseros-passkey-profile-*.provisionprofile) return 0 ;;
    "$RUNNER_TEMP"/browserclaw-passkey-profile-*.provisionprofile) return 0 ;;
    *) return 1 ;;
  esac
}

owned_keychains_file() {
  case "$1" in
    "$RUNNER_TEMP"/browseros-ci-original-keychains-*.txt) return 0 ;;
    *) return 1 ;;
  esac
}

owned_smoke_path() {
  case "$1" in
    "$RUNNER_TEMP"/browseros-ci-codesign-smoke-*) return 0 ;;
    *) return 1 ;;
  esac
}

normalize_keychain_output() {
  sed -e 's/^[[:space:]]*//' -e 's/^"//' -e 's/"$//'
}

require_env() {
  local missing=()
  local name
  for name in "$@"; do
    if [ -z "${!name:-}" ]; then
      missing+=("$name")
    fi
  done
  if [ "${#missing[@]}" -gt 0 ]; then
    echo "::error::Missing required secret(s): ${missing[*]}"
    exit 1
  fi
}

append_env() {
  local path="$1"
  local name="$2"
  local value="$3"
  if [ -n "$path" ]; then
    printf '%s=%s\n' "$name" "$value" >> "$path"
  fi
}

decode_base64_value() {
  local value="$1"
  local output="$2"
  if printf '%s' "$value" | base64 --decode > "$output" 2>/dev/null; then
    return 0
  fi
  printf '%s' "$value" | base64 -D > "$output"
}

decode_certificate() {
  decode_base64_value "$MACOS_CERTIFICATE_P12" "$1"
}

resolve_codesigning_identity() {
  local keychain_path="$1"
  local identities
  local line
  local identity_sha1
  local identity_name
  local match_count=0
  local resolved_sha1=""

  identities="$(security find-identity -v -p codesigning "$keychain_path")"
  while IFS= read -r line; do
    if [[ "$line" =~ ^[[:space:]]*[0-9]+\)[[:space:]]+([[:xdigit:]]{40})[[:space:]]+\"(.*)\"[[:space:]]*$ ]]; then
      identity_sha1="${BASH_REMATCH[1]}"
      identity_name="${BASH_REMATCH[2]}"
      if [ "$identity_name" = "$MACOS_CERTIFICATE_NAME" ]; then
        match_count=$((match_count + 1))
        resolved_sha1="$identity_sha1"
      fi
    fi
  done <<< "$identities"

  case "$match_count" in
    0)
      echo "::error::Imported keychain does not expose the configured macOS signing identity" >&2
      return 1
      ;;
    1)
      printf '%s\n' "$resolved_sha1"
      ;;
    *)
      echo "::error::Imported keychain exposes multiple matching macOS signing identities" >&2
      return 1
      ;;
  esac
}

smoke_codesign_identity() {
  local keychain_path="$1"
  local codesign_identity="$2"
  local smoke_path="$3"

  rm -f "$smoke_path"
  cp /usr/bin/true "$smoke_path"
  chmod u+w "$smoke_path"
  if ! codesign --sign "$codesign_identity" --force --timestamp=none --keychain "$keychain_path" "$smoke_path"; then
    echo "::error::macOS signing identity failed disposable codesign smoke check" >&2
    rm -f "$smoke_path"
    return 1
  fi
  if ! codesign --verify --verbose=2 "$smoke_path"; then
    echo "::error::Disposable macOS codesign smoke verification failed" >&2
    rm -f "$smoke_path"
    return 1
  fi
  rm -f "$smoke_path"
}

cleanup_after_setup_error() {
  local status="$?"
  cleanup_keychain || true
  exit "$status"
}

setup_keychain() {
  if [ -z "${RUNNER_TEMP:-}" ]; then
    echo "::error::RUNNER_TEMP is required"
    exit 1
  fi
  require_owned_state_path
  cleanup_keychain
  require_env \
    MACOS_CERTIFICATE_NAME \
    MACOS_CERTIFICATE_P12 \
    MACOS_CERTIFICATE_PWD \
    MACOS_KEYCHAIN_PASSWORD

  local run_tag="${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}"
  local cert_path="$RUNNER_TEMP/browseros-signing-cert-$run_tag.p12"
  local browseros_profile_path=""
  local browserclaw_profile_path=""
  if [ -n "${PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_B64:-}" ]; then
    browseros_profile_path="$RUNNER_TEMP/browseros-passkey-profile-$run_tag.provisionprofile"
  fi
  if [ -n "${PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_B64:-}" ]; then
    browserclaw_profile_path="$RUNNER_TEMP/browserclaw-passkey-profile-$run_tag.provisionprofile"
  fi
  local keychain_path="$RUNNER_TEMP/browseros-ci-signing-$run_tag.keychain-db"
  local original_keychains_file="$RUNNER_TEMP/browseros-ci-original-keychains-$run_tag.txt"
  local listed_keychains_file="$RUNNER_TEMP/browseros-ci-listed-keychains-$run_tag.txt"
  local smoke_path="$RUNNER_TEMP/browseros-ci-codesign-smoke-$run_tag"
  local original_default_keychain

  security list-keychains -d user 2>/dev/null \
    | normalize_keychain_output > "$listed_keychains_file" || : > "$listed_keychains_file"
  : > "$original_keychains_file"
  local listed_keychain
  while IFS= read -r listed_keychain; do
    if [ -z "$listed_keychain" ]; then
      continue
    fi
    if owned_keychain_path "$listed_keychain"; then
      security lock-keychain "$listed_keychain" >/dev/null 2>&1 || true
      security delete-keychain "$listed_keychain" >/dev/null 2>&1 || rm -f "$listed_keychain"
      continue
    fi
    printf '%s\n' "$listed_keychain" >> "$original_keychains_file"
  done < "$listed_keychains_file"
  rm -f "$listed_keychains_file"

  original_default_keychain="$(
    security default-keychain -d user 2>/dev/null \
      | normalize_keychain_output || true
  )"
  if owned_keychain_path "$original_default_keychain"; then
    security lock-keychain "$original_default_keychain" >/dev/null 2>&1 || true
    security delete-keychain "$original_default_keychain" >/dev/null 2>&1 || rm -f "$original_default_keychain"
    original_default_keychain=""
  fi

  {
    printf 'cert_path=%s\n' "$cert_path"
    printf 'browseros_profile_path=%s\n' "$browseros_profile_path"
    printf 'browserclaw_profile_path=%s\n' "$browserclaw_profile_path"
    printf 'keychain_path=%s\n' "$keychain_path"
    printf 'original_default_keychain=%s\n' "$original_default_keychain"
    printf 'original_keychains_file=%s\n' "$original_keychains_file"
    printf 'smoke_path=%s\n' "$smoke_path"
  } > "$state_path"

  trap cleanup_after_setup_error ERR

  rm -f "$cert_path"
  local profile_path
  for profile_path in "$browseros_profile_path" "$browserclaw_profile_path"; do
    if owned_profile_path "$profile_path"; then
      rm -f "$profile_path"
    fi
  done
  security delete-keychain "$keychain_path" >/dev/null 2>&1 || true
  rm -f "$keychain_path"

  decode_certificate "$cert_path"
  # Each profile authorizes one exact App ID. Decode them to separate,
  # runner-owned paths so a product can never accidentally consume its
  # sibling's authorization document and cleanup can remove both reliably.
  if [ -n "$browseros_profile_path" ]; then
    decode_base64_value "$PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_B64" "$browseros_profile_path"
    chmod 600 "$browseros_profile_path"
  fi
  if [ -n "$browserclaw_profile_path" ]; then
    decode_base64_value "$PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_B64" "$browserclaw_profile_path"
    chmod 600 "$browserclaw_profile_path"
  fi
  security create-keychain -p "$MACOS_KEYCHAIN_PASSWORD" "$keychain_path"
  security set-keychain-settings -lut 21600 "$keychain_path"
  security unlock-keychain -p "$MACOS_KEYCHAIN_PASSWORD" "$keychain_path"
  security import "$cert_path" -P "$MACOS_CERTIFICATE_PWD" -A -t cert -f pkcs12 -k "$keychain_path"
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$MACOS_KEYCHAIN_PASSWORD" "$keychain_path"

  local search_keychains=("$keychain_path")
  local original_keychain
  while IFS= read -r original_keychain; do
    if [ -n "$original_keychain" ]; then
      search_keychains+=("$original_keychain")
    fi
  done < "$original_keychains_file"
  security list-keychains -d user -s "${search_keychains[@]}"
  security default-keychain -d user -s "$keychain_path"
  security show-keychain-info "$keychain_path" >/dev/null

  local codesign_identity
  codesign_identity="$(resolve_codesigning_identity "$keychain_path")"
  if ! smoke_codesign_identity "$keychain_path" "$codesign_identity" "$smoke_path"; then
    cleanup_keychain || true
    return 1
  fi
  rm -f "$cert_path"

  append_env "${GITHUB_ENV:-}" MACOS_CERTIFICATE_NAME "$codesign_identity"
  append_env "${GITHUB_ENV:-}" MACOS_KEYCHAIN_PATH "$keychain_path"
  append_env "${GITHUB_ENV:-}" MACOS_SIGNING_STATE_PATH "$state_path"
  append_env "${GITHUB_ENV:-}" PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_PATH "$browseros_profile_path"
  append_env "${GITHUB_ENV:-}" PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_PATH "$browserclaw_profile_path"
  append_env "${GITHUB_OUTPUT:-}" codesign_identity "$codesign_identity"
  append_env "${GITHUB_OUTPUT:-}" keychain_path "$keychain_path"
  append_env "${GITHUB_OUTPUT:-}" browseros_passkey_profile_path "$browseros_profile_path"
  append_env "${GITHUB_OUTPUT:-}" browserclaw_passkey_profile_path "$browserclaw_profile_path"
  append_env "${GITHUB_OUTPUT:-}" state_path "$state_path"
  trap - ERR
}

cleanup_keychain() {
  if [ -z "$state_path" ] || [ ! -f "$state_path" ]; then
    return 0
  fi
  if ! owned_state_path "$state_path"; then
    echo "::warning::Ignoring unexpected macOS signing state path: $state_path"
    return 0
  fi

  local cert_path=""
  local browseros_profile_path=""
  local browserclaw_profile_path=""
  local keychain_path=""
  local original_default_keychain=""
  local original_keychains_file=""
  local smoke_path=""
  local state_line
  while IFS= read -r state_line; do
    case "$state_line" in
      cert_path=*) cert_path="${state_line#cert_path=}" ;;
      browseros_profile_path=*) browseros_profile_path="${state_line#browseros_profile_path=}" ;;
      browserclaw_profile_path=*) browserclaw_profile_path="${state_line#browserclaw_profile_path=}" ;;
      keychain_path=*) keychain_path="${state_line#keychain_path=}" ;;
      original_default_keychain=*) original_default_keychain="${state_line#original_default_keychain=}" ;;
      original_keychains_file=*) original_keychains_file="${state_line#original_keychains_file=}" ;;
      smoke_path=*) smoke_path="${state_line#smoke_path=}" ;;
    esac
  done < "$state_path"

  local original_keychains=()
  local original_keychain
  if owned_keychains_file "$original_keychains_file" && [ -f "$original_keychains_file" ]; then
    while IFS= read -r original_keychain; do
      if [ -n "$original_keychain" ]; then
        original_keychains+=("$original_keychain")
      fi
    done < "$original_keychains_file"
  fi
  if owned_keychains_file "$original_keychains_file" && [ -f "$original_keychains_file" ]; then
    security list-keychains -d user -s "${original_keychains[@]}" || true
  fi
  if [ -n "$original_default_keychain" ]; then
    security default-keychain -d user -s "$original_default_keychain" || true
  elif [ "${#original_keychains[@]}" -gt 0 ]; then
    security default-keychain -d user -s "${original_keychains[0]}" || true
  fi

  if owned_keychain_path "$keychain_path"; then
    security lock-keychain "$keychain_path" >/dev/null 2>&1 || true
    security delete-keychain "$keychain_path" >/dev/null 2>&1 || rm -f "$keychain_path"
  fi
  if owned_cert_path "$cert_path"; then
    rm -f "$cert_path"
  fi
  local profile_path
  for profile_path in "$browseros_profile_path" "$browserclaw_profile_path"; do
    if owned_profile_path "$profile_path"; then
      rm -f "$profile_path"
    fi
  done
  if owned_smoke_path "$smoke_path"; then
    rm -f "$smoke_path"
  fi
  if owned_keychains_file "$original_keychains_file"; then
    rm -f "$original_keychains_file"
  fi
  rm -f "$state_path"
}

case "${1:-}" in
  setup)
    setup_keychain
    ;;
  cleanup)
    cleanup_keychain
    ;;
  *)
    echo "usage: $0 setup|cleanup" >&2
    exit 2
    ;;
esac
