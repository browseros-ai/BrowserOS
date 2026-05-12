# Compaction Strategy Switching for pi-vcc

**Date:** 2026-05-12
**Status:** Draft
**Scope:** Extend pi-vcc extension to support switching between pi-core (LLM) and pi-vcc (algorithmic) compaction, with per-method configuration.

## Problem

pi-vcc currently has a binary toggle: `overrideDefaultCompaction` (bool). When `false`, only `/pi-vcc` triggers VCC compaction; `/compact` and auto-compact use pi-core's LLM summarization. When `true`, ALL paths use VCC.

There is no way to:
- Use pi-core with a custom summarization prompt
- Configure VCC section caps (transcript window, goal lines, etc.)
- Switch methods per-invocation without editing the config file
- Express "default to VCC but sometimes use pi-core"

## Solution

Extend `pi-vcc-config.json` with a `strategy` field, add sub-commands for per-invocation overrides, and thread tunable caps through the VCC pipeline.

## Config Schema

```jsonc
// ~/.pi/agent/pi-vcc-config.json
{
  // Compaction method
  // "vcc"     → pi-vcc algorithmic (no LLM, deterministic, 30-470ms)
  // "default" → pi-core LLM summarization
  "strategy": "vcc",

  // strategy == "default": appended to pi-core's summarization prompt.
  // null → pi-core's built-in prompt unchanged.
  "defaultCustomPrompt": null,

  // strategy == "vcc": override section caps.
  // null → pi-vcc built-in defaults.
  "vccConfig": {
    "maxTranscriptLines": 120,
    "maxGoalLines": 8,
    "maxFileEntries": 10,
    "maxCommitEntries": 8,
    "maxPreferenceLines": 15,
    "maxOutstandingLines": 10
  },

  // Legacy alias for strategy. Ignored when "strategy" is explicitly set.
  "overrideDefaultCompaction": false,

  "debug": false
}
```

## Strategy Resolution

Priority from highest to lowest:

```
1. Explicit /pi-vcc command           → VCC (always, unchanged)
2. customInstructions sentinel:
   "__pi_vcc__"                       → VCC
   "__default__"                      → pi-core
3. Config strategy field               → "vcc" or "default"
4. Legacy fallback:
   overrideDefaultCompaction == true  → VCC
   overrideDefaultCompaction == false → pi-core (return nothing from hook)
```

When the hook resolves to "default" and `defaultCustomPrompt` is set, the hook cannot inject this into pi-core's summarization prompt (the hook receives the event, not the compact options). Therefore:
- **Per-invocation custom prompt** works via `/pi-vcc:default <prompt>` command (passes `customInstructions` through `ctx.compact()`)
- **Auto-compact with custom prompt** is NOT supported — auto-compact always uses pi-core's built-in prompt. `defaultCustomPrompt` is a config-time hint only usable from commands.

## Slash Commands

| Command | Method | Notes |
|---------|--------|-------|
| `/pi-vcc` | VCC | Existing behavior, unchanged |
| `/pi-vcc:default` | pi-core | One-shot override, no custom prompt |
| `/pi-vcc:default <prompt>` | pi-core | One-shot override with custom prompt |
| `/pi-vcc:vcc` | VCC | Explicit alias for `/pi-vcc` |
| `/pi-vcc:config` | — | Print current config to notification |

`/pi-vcc:default` works by calling `ctx.compact({})` without the `__pi_vcc__` sentinel. The `session_before_compact` hook sees no sentinel, resolves strategy from config or defaults to letting pi-core handle it.

`/pi-vcc:default <prompt>` calls `ctx.compact({ customInstructions: <prompt> })`. The hook sees non-sentinel instructions and returns nothing — pi-core uses the prompt.

## Hook Logic

```ts
// In before-compact.ts
pi.on("session_before_compact", (event, ctx) => {
  const { preparation, branchEntries, customInstructions } = event;
  const settings = loadSettings();

  const isExplicitVcc = customInstructions === PI_VCC_COMPACT_INSTRUCTION;

  // 1. Explicit /pi-vcc always uses VCC
  if (isExplicitVcc) {
    return runVccCompaction(event, settings, branchEntries);
  }

  // 2. Resolve effective strategy
  const strategy = settings.strategy
    ?? (settings.overrideDefaultCompaction ? "vcc" : "default");

  // 3. VCC strategy
  if (strategy === "vcc") {
    return runVccCompaction(event, settings, branchEntries);
  }

  // 4. Default strategy — let pi-core handle it
  //    (returning nothing = pi-core proceeds with LLM summarization)
  //    Note: defaultCustomPrompt is only injectable from commands,
  //    not from the hook. Auto-compact always uses pi-core's built-in prompt.
  return;
});
```

`runVccCompaction()` extracts the existing VCC logic into a named function. No behavioral change.

## VCC Pipeline Overrides

New interface:

```ts
export interface VccOverrides {
  maxTranscriptLines?: number;   // default: 120
  maxGoalLines?: number;         // default: 8
  maxFileEntries?: number;       // default: 10
  maxCommitEntries?: number;     // default: 8
  maxPreferenceLines?: number;   // default: 15
  maxOutstandingLines?: number;  // default: 10
}
```

Threading:
- `settings.vccConfig` → passed to `compile()` via `CompileInput.overrides`
- `compile()` → passes to `formatSummary()` via `FormatOptions.overrides`
- `formatSummary()` → passes caps to `capBrief()` and section formatters
- Every function uses `overrides?.field ?? BUILT_IN_DEFAULT` — no behavior change when overrides absent

## File Changes

| File | Change |
|------|--------|
| `src/core/settings.ts` | Add `strategy`, `defaultCustomPrompt`, `vccConfig` to `PiVccSettings`; update `DEFAULT_SETTINGS` and `scaffoldSettings()` |
| `src/types.ts` | Add `VccOverrides` interface |
| `src/hooks/before-compact.ts` | Extract `runVccCompaction()`; add strategy routing in hook |
| `src/core/summarize.ts` | Add `overrides?: VccOverrides` to `CompileInput`; thread to `formatSummary()` |
| `src/core/format.ts` | Accept overrides in `formatSummary()` and `capBrief()`; use override values with built-in fallbacks |
| `src/commands/pi-vcc.ts` | Add sub-command parsing for `:default`, `:default <prompt>`, `:vcc`, `:config` |

## Backwards Compatibility

| Existing Config | Behavior After Change |
|----------------|----------------------|
| `overrideDefaultCompaction: false` (default) | `strategy` unset → resolves to "default" → pi-core handles `/compact` and auto-compact |
| `overrideDefaultCompaction: true` | `strategy` unset → resolves to "vcc" → VCC handles everything |
| `strategy: "vcc"` + `overrideDefaultCompaction: false` | `strategy` wins → VCC |
| `/pi-vcc` command | Always VCC, regardless of config |
| `vccConfig: null` | All caps use built-in defaults — no change |

## Testing

- Unit: `resolveStrategy()` with all combinations of sentinel, strategy, legacy flag
- Unit: `runVccCompaction()` produces identical output to existing hook logic
- Unit: `formatSummary()` with and without `VccOverrides`
- Unit: `scaffoldSettings()` migrates old configs correctly
- Integration: `/pi-vcc:default` triggers pi-core compaction (not VCC)
- Integration: `/pi-vcc:default <prompt>` passes custom prompt to pi-core
