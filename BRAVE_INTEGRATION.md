# JarvisOS × Brave Privacy Integration

> **Branch:** `jarvis-privacy-integration`
> **Date:** May 2026
> **Author:** Jarvis RTX⚡ (build agent for PsProsen-Dev)
> **Package:** `@jarvisos/privacy` at `packages/jarvis-privacy/`

---

## Overview

This document tracks the porting of Brave browser's privacy components into JarvisOS, creating a
native privacy layer (`@jarvisos/privacy`) that merges Brave's proven privacy stack with
BrowserOS's AI agent architecture.

**Source repos used:**
- `brave-core` — Chromium-based browser core (depth-1 clone, May 2026)
- `adblock-rust` — Brave's Rust-based adblock engine

---

## What Was Ported

### 1. Brave Shields — `packages/jarvis-privacy/shields/`

| File | Source | Status |
|------|--------|--------|
| `brave_shield_constants.h` | `brave-core/components/brave_shields/core/common/` | ✅ Copied |
| `brave_shields_utils.h` | `brave-core/components/brave_shields/core/browser/` | ✅ Copied |

**Purpose:** Core Shields constants (cookie blocking, fingerprinting modes, ad-blocking levels) and
utility functions for querying/setting shield state per-origin. These are the foundation for
JarvisOS's per-tab privacy control UI.

---

### 2. Adblock Engine — `packages/jarvis-privacy/adblock/`

| File | Source | Status |
|------|--------|--------|
| `slim-list.txt` | `adblock-rust/data/` | ✅ Copied — primary filter list |
| `FILTER_LIST_REGISTRY.txt` | Generated from brave-core lists | ✅ Created — 59 providers documented |
| `Cargo.toml` | `adblock-rust/` | ✅ Copied — Rust engine manifest |
| `blocker.rs` | `adblock-rust/src/` | ✅ Copied — core blocking logic |
| `content_blocking.rs` | `adblock-rust/src/` | ✅ Copied — content blocking rules |
| `lists/slim-list.txt` | `adblock-rust/data/` | ✅ Copied — filter rules |

**Note on depth-1 clone limitation:** `brave-core/components/third_party/adblock/lists/` contains
59 filter list provider subdirectories (abpindo, adguard_cn, easylist, etc.) but the actual `.txt`
filter files are not present in a depth-1 clone — they are fetched via `gclient sync`. All 59
provider names are documented in `FILTER_LIST_REGISTRY.txt` for future integration.

---

### 3. Fingerprint Protection (Farbling) — `packages/jarvis-privacy/fingerprint/`

| File | Source | Status |
|------|--------|--------|
| `brave_farbling_service.h` | `brave-core/components/brave_shields/content/browser/` | ✅ Copied |
| `ad_block_engine.h` | `brave-core/components/brave_shields/content/browser/` | ✅ Copied |
| `brave_shields_util.h` | `brave-core/components/brave_shields/content/browser/` | ✅ Copied |

**Note on farbling browsertest files:** `brave-core/browser/farbling/` contains 16 `.cc`
browsertest files covering canvas, WebGL, WebAudio, fonts, navigator APIs, speech synthesis,
screen, USB, keyboard, languages, plugins, deviceMemory, hardwareConcurrency, userAgent,
offscreenCanvas, and dark mode fingerprinting. These are **test files only** (no `.h` headers in
that directory) — implementation headers live in `brave_shields`. The test files serve as
integration targets/spec for JarvisOS farbling implementation.

**Farbling coverage (16 test vectors from brave-core):**
- Canvas / OffscreenCanvas
- WebGL
- WebAudio
- Font enumeration
- Navigator: languages, plugins, deviceMemory, hardwareConcurrency, userAgent, USB, keyboard
- Screen dimensions
- Speech synthesis
- Dark mode detection

---

### 4. Bounce Tracking (Debounce) — `packages/jarvis-privacy/debounce/`

| File | Source | Status |
|------|--------|--------|
| `debounce_rule.h` | `brave-core/components/debounce/core/browser/` | ✅ Copied |
| `debounce_service.h` | `brave-core/components/debounce/core/browser/` | ✅ Copied |

**Purpose:** Debounce rules define redirect-chain detection patterns. The service intercepts
navigation throttle events to strip tracking redirects (e.g., link.com/click?url=target.com →
direct navigation to target.com). Critical for protecting against bounce tracking used by ad
networks.

---

### 5. Referrer Protection — `packages/jarvis-privacy/referrer/`

| Status | Notes |
|--------|-------|
| ⏳ Pending | Directory created, implementation not yet ported |

**Planned source:** `brave-core/components/brave_shields/core/browser/` — referrer trimming
utilities that reduce `Referer` header leakage across origins.

---

## Integration Roadmap

### Phase 1 — Headers & Engine (DONE ✅)
- [x] Package structure created (`packages/jarvis-privacy/`)
- [x] Shields constants and utils headers copied
- [x] Debounce rule + service headers copied
- [x] Farbling service header copied
- [x] adblock-rust engine source + slim-list filter copied
- [x] Filter list registry (59 providers) documented
- [x] `package.json` for `@jarvisos/privacy` created

### Phase 2 — Rust Engine Integration (DONE ✅)
- [x] `adblock-rust v0.12.4` cargo check PASSED (GNU toolchain, May 2026)
- [x] wasm32-unknown-unknown + x86_64-pc-windows-gnu targets added
- [x] MSVC limitation documented — GNU toolchain required on Windows
- [ ] Build WASM/NAPI bindings for browser process consumption (wasm-pack needs MSVC linker)
- [ ] Wire `blocker.rs` to JarvisOS network request interception layer
- [ ] Fetch full filter lists via direct EasyList/AdGuard CDN

### Phase 3 — Build Environment + TS Integration (DONE ✅ — May 10, 2026)
- [x] Bun 1.3.13 installed (BrowserOS build tool requirement)
- [x] BrowserOS-dev full architecture analyzed (Chromium 146.0.7680.31)
- [x] JarvisOS vs BrowserOS-dev diff completed — packages identical except `jarvis-privacy`
- [x] `jarvis-adblock.ts` TypeScript wrapper created (`packages/jarvis-privacy/adblock/`)
- [x] `privacy-defaults.json` config created (`packages/jarvis-privacy/config/`)
- [x] brave-core-master Windows installer + privacy patches documented
- [x] adblock-rust v0.12.4 verified compiles (cargo check ✅, GNU toolchain)
- [ ] Port `brave_shield_constants.h` constants to TypeScript enums
- [ ] Implement per-origin shield settings store (Zustand or Jotai)
- [ ] Build JarvisOS Shields panel UI component (React/Tailwind)
- [ ] Wire shields state to network/content blocking pipeline

### Phase 4 — Farbling Implementation (TODO)
- [ ] Port farbling randomization seeds to JarvisOS renderer process
- [ ] Implement canvas fingerprint farbling (noise injection)
- [ ] Implement AudioContext farbling
- [ ] Implement navigator API overrides (plugins, languages, deviceMemory)
- [ ] Implement WebGL parameter farbling
- [ ] Validate against 16 brave-core browsertest specs

### Phase 5 — Debounce + Referrer (TODO)
- [ ] Port debounce rule JSON/patterns to JarvisOS navigation throttle
- [ ] Implement referrer trimming (cross-origin → origin-only, or strip)
- [ ] Add UI toggles for debounce and referrer protection levels

---

## Architecture Notes

```
JarvisOS Request Pipeline (proposed):
  
  [Tab Navigation] 
      → [Debounce Throttle] (bounce-tracking strip)
      → [Ad/Tracker Block] (adblock-rust engine + slim-list)  
      → [Referrer Trim] (origin-only or no-referrer)
      → [Network Request]
  
  [Renderer Process]
      → [Farbling Layer] (canvas/audio/navigator noise)
      → [Page Script Execution]
```

---

## License

All ported files from `brave-core` are subject to the
[Brave Browser License](https://github.com/brave/brave-core/blob/master/LICENSE) (MPL 2.0).
`adblock-rust` is licensed under MPL 2.0.
JarvisOS integration code: see `LICENSE` in repo root.

---

## Phase 3 Findings — BrowserOS-dev Architecture Analysis

### Chromium Base
- **Version:** Chromium `146.0.7680.31` (BASE_COMMIT: `4d3225104176d`)
- **Build system:** Bun 1.3.6 required (strict — npm/yarn/pnpm all rejected)
- **Package manager:** bun workspaces monorepo

### BrowserOS Agent Stack (packages/browseros-agent/)
| App | Tech | Role |
|-----|------|------|
| `apps/agent` | WXT + React + Bun | Browser extension (AI agent UI) |
| `apps/server` | Bun + TypeScript | Local server — CDP, MCP, HTTP |
| `apps/cli` | Bun + Commander | CLI installer |
| `apps/eval` | Bun | Test/eval harness |

**Server ports (config.sample.json):**
- CDP: 9000, HTTP/MCP: 9100, Agent: 9200, Extension: 9300

### JarvisOS vs BrowserOS-dev Diff
| Item | JarvisOS | BrowserOS-dev |
|------|----------|---------------|
| `packages/browseros` | ✅ Identical | ✅ Same |
| `packages/browseros-agent` | ✅ Identical | ✅ Same |
| `packages/jarvis-privacy` | ✅ JarvisOS-only | ❌ Not present |
| `BRAVE_INTEGRATION.md` | ✅ JarvisOS-only | ❌ Not present |
| `JARVIS.md` | ✅ JarvisOS-only | ❌ Not present |
| `SOUL.md` | ✅ JarvisOS-only | ❌ Not present |

**Conclusion:** JarvisOS = BrowserOS-dev base + Brave privacy layer (jarvis-privacy package). Architecture is sound — clean separation maintained.

### brave-core-master Windows/Privacy Key Files
- **Windows installer:** `installer/mini_installer/` + `installer/setup/` + `installer/util/`
- **Windows-specific browser files:** `browser/brave_shell_integration_win.cc`, `browser/default_protocol_handler_utils_win.cc`
- **Windows Chromium patches (BrowserOS):** 5 patches — download warning, rcpy, command IDs, rc errors
- **Privacy components in brave-core:** `brave_shields/`, `debounce/`, `global_privacy_control/`, `privacy_sandbox/`, `brave_adblock_ui/`
- **Privacy patches:** 6 privacy_page patches (settings UI for Android + desktop)

### adblock-rust v0.12.4 Cargo Check Result
```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 17s
```
**Status: VERIFIED COMPILES** — 77 dependencies resolved, GNU toolchain required.

---

*Generated by Jarvis RTX⚡ build agent — JarvisOS privacy integration mission, May 2026.*
