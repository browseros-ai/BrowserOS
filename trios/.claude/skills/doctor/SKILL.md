---
name: doctor
description: HEALER — diagnose trios build, heal dirty files, monitor health. Every loop = action + proof.
argument-hint: [quick|full|scan|build|commit] [lang:ru|en]
allowed-tools: Bash(swiftc *), Bash(git *), Bash(ls *), Bash(find *), Bash(grep *), Bash(wc *), Bash(cat *), Bash(date *), Bash(tail *), Bash(echo *), Read, Edit, Write
---

## HEALER MODE — DIAGNOSE → HEAL → REPORT

**HONESTY RULE**: Never say "all good" if there are dirty files. Fix them or explain WHY.

## Healing Protocol

### Step 1: DIAGNOSE
```bash
cd /Users/playra/BrowserOS-full/trios
git status --porcelain                    # dirty files
git diff --name-only -- '*.swift'        # changed swift files
./build.sh 2>&1 | tail -10              # build check
curl -s http://127.0.0.1:9105/health    # MCP server
cat .trinity/doctor_prev.dat 2>/dev/null || echo "no prev state"
```

### Step 2: HEAL

**2a. Build broken?** → Read errors, fix code, ./build.sh again
**2b. Dirty .swift files?** → Build passes, then commit:
```bash
git add <dirty .swift files>
git commit -m "ring-NNN-fix: description (Closes #N)"
```
**2c. Dirty state files?** (.trinity/*, .claude/*) → Batch commit
**2d. Build script stale?** → Update build.sh if new files added

### Step 3: VERIFY
```bash
git status --porcelain
git log --oneline -3
```

### Step 4: SNAPSHOT
```bash
echo "$(date +%s) build=$(test -f trios_app && echo OK || echo FAIL) dirty=$(git status --porcelain | wc -l | tr -d ' ')" > .trinity/doctor_prev.dat
```

## Report Format
```
## TRI Doctor Report

**Status: {FIXED|PARTIAL|FAILED}**

### Diagnosis
- Build: {PASS|FAIL}
- Dirty files: {N}
- Server: {UP|DOWN}

### Treatment
- File: {path}
- Change: {what changed}
- Verification: build {PASS|FAIL}

### Remaining Issues
- {list or "None — all healthy"}
``
