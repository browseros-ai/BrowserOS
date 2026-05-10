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

### Phase 2 — Rust Engine Integration (TODO)
- [ ] Integrate `adblock-rust` as a Cargo workspace dependency
- [ ] Build WASM/NAPI bindings for browser process consumption
- [ ] Wire `blocker.rs` to JarvisOS network request interception layer
- [ ] Fetch full filter lists via `gclient sync` or direct EasyList/AdGuard CDN

### Phase 3 — Shields UI Layer (TODO)
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

*Generated by Jarvis RTX⚡ build agent — JarvisOS privacy integration mission, May 2026.*
