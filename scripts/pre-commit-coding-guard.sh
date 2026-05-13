#!/bin/sh
#====================================================================
# pre-commit gate — BrowserOS coding-guard
#
# Called by lefthook with staged TS/TSX files as arguments.
# Also works standalone (reads git diff --cached if no args).
#
# Gates (7):
#   Gate 1: ast-grep scan — blocks on error severity
#   Gate 2: No stub implementations (blocking)
#   Gate 3: No explicit any (warning)
#   Gate 4: Unsafe JSON.parse without try-catch (blocking)
#   Gate 5: Silent .catch(() => {}) (warning)
#   Gate 6: Math.random() for IDs (warning)
#   Gate 7: Lossy JSON clone (warning)
#====================================================================
set -e

ROOT_DIR="$(git rev-parse --show-toplevel)"
SG="${SG:-sg}"

errors=0
warnings=0

# ── Get staged files ──────────────────────────────────────────────
# Lefthook passes files as args. Standalone mode reads git diff.
if [ $# -gt 0 ]; then
  # Lefthook mode: files as arguments
  staged_files=""
  for f in "$@"; do
    # Strip leading ./ if present
    f_clean="${f#./}"
    staged_files="$staged_files $f_clean"
  done
else
  # Standalone mode: read from git
  staged_files=$(git diff --cached --name-only --diff-filter=ACM 2>/dev/null | \
    grep -E '\.(ts|tsx)$' || true)
fi

# Filter: non-test, non-generated, non-vendor
staged_files=$(echo "$staged_files" | tr ' ' '\n' | \
  grep -v '\.test\.' | \
  grep -v '\.spec\.' | \
  grep -v '/test/' | \
  grep -v '/tests/' | \
  grep -v '/__tests__/' | \
  grep -v '/node_modules/' | \
  grep -v '/dist/' | \
  grep -v '\.d\.ts$' | \
  grep -v '^$' || true)

if [ -z "$staged_files" ]; then
  exit 0
fi

echo "[coding-guard] Scanning $(echo "$staged_files" | wc -l | tr -d ' ') file(s)..."

# ── Gate 1: ast-grep with deployed rules ──────────────────────────
if [ -d "$ROOT_DIR/.sg-rules" ] && [ -f "$ROOT_DIR/sgconfig.yml" ]; then
  # Build absolute file list
  tmpfiles=""
  for f in $staged_files; do
    target="$ROOT_DIR/$f"
    [ -f "$target" ] && tmpfiles="$tmpfiles $target"
  done

  if [ -n "$tmpfiles" ]; then
    set +e
    scan_output=$($SG scan $tmpfiles --json 2>/dev/null)
    scan_exit=$?
    set -e

    if [ $scan_exit -ne 0 ] && [ $scan_exit -ne 1 ]; then
      echo "[coding-guard] ⚠️  sg scan failed (exit=$scan_exit) — skipping Gate 1"
    elif [ -n "$scan_output" ]; then
      error_count=$(echo "$scan_output" | python3 -c "
import json,sys
data = json.load(sys.stdin)
errors = [m for m in data if any(
    r.get('severity') == 'error'
    for r in [m.get('rule', {})]
)]
warnings_list = [m for m in data if any(
    r.get('severity') in ('warning', 'info')
    for r in [m.get('rule', {})]
)]
for e in errors:
    file = e.get('file','?')
    line = e.get('range',{}).get('start',{}).get('line','?')
    msg = e.get('message','?')
    print(f'  ❌ {file}:{line} — {msg}')
for w in warnings_list:
    file = w.get('file','?')
    line = w.get('range',{}).get('start',{}).get('line','?')
    msg = w.get('message','?')
    print(f'  ⚠️  {file}:{line} — {msg}')
print(f'{len(errors)}')
" 2>/dev/null || echo "0")

      error_num=$(echo "$error_count" | tail -1)
      if [ "$error_num" -gt 0 ] 2>/dev/null; then
        errors=$((errors + error_num))
      fi
    fi
  fi
else
  echo "[coding-guard] No .sg-rules/ found — skipping Gate 1"
fi

# ── Gate 2: Stub implementations (blocking) ───────────────────────
for f in $staged_files; do
  target="$ROOT_DIR/$f"
  [ -f "$target" ] || continue
  stubs=$(grep -inE '(throw new .*Error.*not implement|console\.\w+.*stub|console\.\w+.*placeholder|console\.\w+.*todo.*implement)' "$target" 2>/dev/null || true)
  if [ -n "$stubs" ]; then
    echo "[coding-guard] ❌ Stub in $f:"
    echo "$stubs" | head -5
    errors=$((errors + 1))
  fi
done

# ── Gate 3: Explicit any (warning) ────────────────────────────────
for f in $staged_files; do
  target="$ROOT_DIR/$f"
  [ -f "$target" ] || continue
  anys=$(grep -nE ':\s*any\b' "$target" 2>/dev/null | \
    grep -v 'any\[\]' | \
    grep -v 'Record<string, any>' | \
    grep -v '// eslint-disable' | \
    grep -v 'as any' || true)
  if [ -n "$anys" ]; then
    echo "[coding-guard] ⚠️  Explicit any in $f:"
    echo "$anys" | head -5
    warnings=$((warnings + 1))
  fi
done

# ── Gate 4: Unsafe JSON.parse (blocking) ──────────────────────────
for f in $staged_files; do
  target="$ROOT_DIR/$f"
  [ -f "$target" ] || continue
  unsafe=$(awk '/try[[:space:]]*\{/{try_block=1} /\}/{if(try_block) try_block=0} /JSON\.parse/ && !try_block{print NR": "$0}' "$target" 2>/dev/null || true)
  if [ -n "$unsafe" ]; then
    echo "[coding-guard] ❌ Unsafe JSON.parse in $f:"
    echo "$unsafe" | head -5
    errors=$((errors + 1))
  fi
done

# ── Gate 5: Silent .catch (warning) ───────────────────────────────
for f in $staged_files; do
  target="$ROOT_DIR/$f"
  [ -f "$target" ] || continue
  silent=$(grep -nE '\.catch\(\(\) => (\{\}|null)\)' "$target" 2>/dev/null | \
    grep -v 'track\|analytics\|telemetry' || true)
  if [ -n "$silent" ]; then
    echo "[coding-guard] ⚠️  Silent .catch() in $f:"
    echo "$silent" | head -5
    warnings=$((warnings + 1))
  fi
done

# ── Gate 6: Math.random() for IDs (warning) ───────────────────────
for f in $staged_files; do
  target="$ROOT_DIR/$f"
  [ -f "$target" ] || continue
  badid=$(grep -nE 'Math\.random\(\)\.toString' "$target" 2>/dev/null || true)
  if [ -n "$badid" ]; then
    echo "[coding-guard] ⚠️  Math.random() for ID in $f — use crypto.randomUUID():"
    echo "$badid" | head -3
    warnings=$((warnings + 1))
  fi
done

# ── Gate 7: Lossy JSON clone (warning) ────────────────────────────
for f in $staged_files; do
  target="$ROOT_DIR/$f"
  [ -f "$target" ] || continue
  clone=$(grep -nE 'JSON\.parse\(JSON\.stringify' "$target" 2>/dev/null || true)
  if [ -n "$clone" ]; then
    echo "[coding-guard] ⚠️  Lossy JSON clone in $f — use structuredClone():"
    echo "$clone" | head -3
    warnings=$((warnings + 1))
  fi
done

# ── Result ────────────────────────────────────────────────────────
if [ "$errors" -gt 0 ]; then
  echo ""
  echo "[coding-guard] ❌ BLOCKED — $errors error(s), $warnings warning(s)"
  exit 1
elif [ "$warnings" -gt 0 ]; then
  echo "[coding-guard] ⚠️  $warnings warning(s) — review recommended"
fi

echo "[coding-guard] ✅ All 7 gates passed"
