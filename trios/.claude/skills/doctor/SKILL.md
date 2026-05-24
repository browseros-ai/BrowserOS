---
name: doctor
description: HEALER — diagnose trios build, heal dirty files, monitor health. No .sh/.py scripts per L7 UNITY.
argument-hint: [quick|full|scan|build|commit] [lang:ru|en]
allowed-tools: fs_read, fs_write, fs_edit, shell_execute, fs_list
---

## HEALER MODE — DIAGNOSE → HEAL → REPORT

**HONESTY RULE**: Never say all good if dirty files exist. Fix or explain WHY.

**L7 UNITY**: No ad-hoc .sh/.py scripts. Use MCP tools only.

## Healing Protocol

### Step 1: DIAGNOSE (via MCP tools)

```
shell_execute: "cd /Users/playra/BrowserOS-full/trios && git status --porcelain | wc -l"
shell_execute: "curl -s http://127.0.0.1:9105/health"
shell_execute: "cd /Users/playra/BrowserOS-full/trios && ./build.sh 2>&1 | tail -5"
fs_read: "/Users/playra/BrowserOS-full/trios/.trinity/doctor_prev.dat"
```

### Step 2: HEAL

**2a. Build broken?** → fs_read errors, fs_edit fix, shell_execute rebuild
**2b. Dirty .swift?** → Commit via shell_execute:
```
shell_execute: "cd /Users/playra/BrowserOS-full/trios && git add -A && git commit -m ring-NNN-fix: desc (Closes #N)"
```
**2c. Dirty state?** → Batch commit via shell_execute
**2d. Build script stale?** → fs_edit build.sh

### Step 3: VERIFY
```
shell_execute: "cd /Users/playra/BrowserOS-full/trios && git status --porcelain && git log --oneline -3"
```

### Step 4: SNAPSHOT
```
fs_write: path=.trinity/doctor_prev.dat, content=timestamp build_status dirty_count
```

## Trinity Compliance
- L1 TRACEABILITY: ring-NNN-type: desc (Closes #N)
- L2 GENERATION: No hand-editing generated code
- L3 PURITY: ASCII-only identifiers
- L4 TESTABILITY: Build passes after heal
- L7 UNITY: No .sh/.py scripts

## Report Format
```
## TRI Doctor Report

**Status: {FIXED|PARTIAL|FAILED}**

### Diagnosis
- Build: {PASS|FAIL}
- Dirty: {N} files
- Server: {UP|DOWN}

### Treatment
- File: {path}
- Change: {what}
- Verification: build {PASS|FAIL}

### Remaining
- {list or None — all healthy}
```
