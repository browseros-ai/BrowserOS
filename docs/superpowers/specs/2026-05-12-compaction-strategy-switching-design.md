# BrowserOS Compaction Strategy Switching

**Date:** 2026-05-12
**Status:** Draft
**Scope:** Augment BrowserOS compaction with VCC (algorithmic) method alongside existing LLM summarization. Support method switching and per-method configuration.

**Skills invoked:** `brainstorming`

## Problem

BrowserOS compaction is a `prepareStep` in `ToolLoopAgent` that cascades: prune → reduceToolOutputs → compactMessages. The `compactMessages()` function always calls the LLM to summarize history. There is no way to:

- Use an algorithmic (no-LLM) compaction method like pi-vcc
- Customize the summarization prompt for the default LLM method
- Tune VCC section caps (transcript window, goal lines, etc.)
- Switch methods via configuration

## Solution

Add a `compaction` config block to `ResolvedAgentConfig`. Route inside `compactMessages()` based on `method`. Vendor the pi-vcc algorithmic pipeline (normalize → filter → sections → format) adapted for AI SDK `ModelMessage[]`.

## Data Shapes

These are the exact types flowing through the compaction pipeline. Any adapter or new code must conform to these.

### ModelMessage (AI SDK)

```ts
type ModelMessage = SystemModelMessage | UserModelMessage | AssistantModelMessage | ToolModelMessage

interface SystemModelMessage {
  role: 'system'
  content: string
  providerOptions?: ProviderOptions
}

interface UserModelMessage {
  role: 'user'
  content: UserContent  // string | Array<TextPart | ImagePart | FilePart>
  providerOptions?: ProviderOptions
}

interface AssistantModelMessage {
  role: 'assistant'
  content: AssistantContent  // string | Array<TextPart | FilePart | ReasoningPart | ToolCallPart | ToolResultPart | ToolApprovalRequest>
  providerOptions?: ProviderOptions
}

interface ToolModelMessage {
  role: 'tool'
  content: ToolContent  // Array<ToolResultPart | ToolApprovalResponse>
  providerOptions?: ProviderOptions
}
```

### CompactionState (carried across turns)

```ts
interface CompactionState {
  existingSummary: string | null
  compactionCount: number
}
```

Passed via `experimental_context` in the `prepareStep` callback. Persisted across turns by the AI SDK agent loop. On first compaction, `existingSummary` is `null` and `compactionCount` is `0`. After each successful compaction, `existingSummary` holds the last summary text and `compactionCount` increments.

### ComputedConfig (derived from contextWindow)

```ts
interface ComputedConfig {
  contextWindow: number            // e.g. 200000
  reserveTokens: number            // 20000 (or 50% for small contexts)
  triggerRatio: number             // e.g. 0.9
  triggerThreshold: number         // contextWindow - reserveTokens
  keepRecentTokens: number         // ~35% of threshold, capped at 20000
  minSummarizableTokens: number    // minimum content worth summarizing
  maxSummarizationInput: number    // cap on input to summarizer
  summarizerMaxOutputTokens: number // budget for summary output
  summarizationTimeoutMs: number   // abort timeout for LLM call
  fixedOverhead: number            // 12000 tokens
  safetyMultiplier: number         // 1.3x
  imageTokenEstimate: number       // 1000 per image
  toolOutputMaxChars: number       // truncation limit
}
```

### StepWithUsage

```ts
interface StepWithUsage {
  usage?: {
    inputTokens?: number | undefined
    outputTokens?: number | undefined
  }
}
```

### SplitPointResult

```ts
interface SplitPointResult {
  splitIndex: number       // -1 if no safe split found
  turnStartIndex: number   // -1 if not a split turn
  isSplitTurn: boolean     // true when split lands mid-turn
}
```

### prepareStep Signature

```ts
type PrepareStep = (options: {
  messages: ModelMessage[]
  steps: ReadonlyArray<StepWithUsage>
  model: LanguageModel
  experimental_context: unknown  // carries CompactionState
}) => Promise<{
  messages: ModelMessage[]
  experimental_context: unknown  // updated CompactionState
}>
```

### Compaction Output Format

Regardless of method (LLM or VCC), `compactMessages()` always returns:

```ts
[
  { role: 'user', content: `${summary}\n\nContinue from where you left off.` },
  ...toKeep  // the recent messages after the split point
]
```

The summary is injected as a synthetic `user` message. The `toKeep` tail is untouched.

## Current Architecture

```
ai-sdk-agent.ts
  → createCompactionPrepareStep({ contextWindow })
    → prepareStep({ messages, steps, model, experimental_context })
      → getCurrentTokenCount → stripBinaryContent → pruneMessages → reduceToolOutputs
      → compactMessages(model, messages, config, state)
        → findSafeSplitPoint → summarizeMessages (LLM) → [summary_msg + kept_tail]
        → fallback: slidingWindow (drop oldest)
```

Key files:
- `apps/server/src/agent/compaction.ts` — `compactMessages()`, `createCompactionPrepareStep()`
- `apps/server/src/agent/compaction/prompt.ts` — `buildSummarizationPrompt()`, `messagesToTranscript()`
- `apps/server/src/agent/compaction/utils.ts` — token estimation, split points, sliding window
- `apps/server/src/agent/compaction/content.ts` — binary stripping, tool output formatting
- `apps/server/src/agent/ai-sdk-agent.ts` — wires `compactionPrepareStep` into `ToolLoopAgent`

## Config Schema

New optional field on `ResolvedAgentConfig`:

```ts
export interface CompactionStrategyConfig {
  /** Compaction method. Default: "default" (LLM summarization) */
  method: "default" | "vcc"

  /**
   * When method=="default": replace the summarization prompt.
   * null/undefined = use built-in BrowserOS prompt (Goal/Progress/Next Steps).
   * Non-null = user's custom prompt replaces the SUMMARY_FORMAT block.
   */
  customPrompt?: string

  /**
   * When method=="vcc": override section caps.
   * null/undefined = use built-in pi-vcc defaults.
   */
  vccConfig?: {
    maxTranscriptLines?: number    // default: 120
    maxGoalLines?: number          // default: 8
    maxFileEntries?: number        // default: 10
    maxCommitEntries?: number      // default: 8
    maxPreferenceLines?: number    // default: 15
    maxOutstandingLines?: number   // default: 10
  }
}
```

Add to `ResolvedAgentConfig`:
```ts
compaction?: CompactionStrategyConfig
```

Thread from server config through to `createCompactionPrepareStep()`.

## compactMessages() Changes

### Before (current)
```
split → summarizeMessages(model, ...) → [summary_msg + kept_tail]
fallback → slidingWindow
```

### After
```
split → resolveMethod(config.compaction)
  method == "default" (no customPrompt) → summarizeMessages(model, ...) [UNCHANGED]
  method == "default" + customPrompt → summarizeMessages(model, ..., customPrompt) [MODIFIED]
  method == "vcc" → vccCompile(messages, existingSummary, vccConfig) → [summary_msg + kept_tail] [NEW]
any → fallback: slidingWindow [UNCHANGED]
```

The return type is always `ModelMessage[]` regardless of method. VCC output gets wrapped in a `{ role: "user", content: summary + "\n\nContinue from where you left off." }` message — same format as LLM path.

### Pseudocode

```ts
async function compactMessages(
  model: LanguageModel,
  messages: ModelMessage[],
  config: ComputedConfig,
  state: CompactionState,
  compactionConfig?: CompactionStrategyConfig,  // NEW
): Promise<ModelMessage[]> {
  // ... split point logic (unchanged) ...

  let summary: string | null = null;

  const method = compactionConfig?.method ?? "default";

  if (method === "vcc") {
    // VCC path — no LLM call
    summary = vccCompile(toSummarize, state.existingSummary, compactionConfig?.vccConfig);

    // Handle turn prefix same as LLM path but with VCC
    if (isSplitTurn && summarizedTurnPrefix.length > 0) {
      const prefixSummary = vccCompile(summarizedTurnPrefix, null, compactionConfig?.vccConfig);
      if (summary && prefixSummary) {
        summary = `${summary}\n\n---\n\n**Turn Context (split turn):**\n\n${prefixSummary}`;
      } else {
        summary = summary ?? prefixSummary;
      }
    }
  } else {
    // Default LLM path (unchanged, plus customPrompt support)
    const customPrompt = compactionConfig?.customPrompt;

    if (isSplitTurn && summarizedTurnPrefix.length > 0) {
      if (toSummarize.length > 0) {
        const [historySummary, turnPrefixSummary] = await Promise.all([
          summarizeMessages(model, toSummarize, state.existingSummary, timeout, maxOutput, customPrompt),
          summarizeTurnPrefix(model, summarizedTurnPrefix, timeout, prefixBudget),
        ]);
        summary = (historySummary && turnPrefixSummary)
          ? `${historySummary}\n\n---\n\n**Turn Context (split turn):**\n\n${turnPrefixSummary}`
          : historySummary ?? turnPrefixSummary;
      } else {
        summary = await summarizeTurnPrefix(model, summarizedTurnPrefix, timeout, prefixBudget);
      }
    } else {
      summary = await summarizeMessages(model, toSummarize, state.existingSummary, timeout, maxOutput, customPrompt);
    }
  }

  // ... rest unchanged (null check, size check, sliding window fallback) ...
}
```

## VCC Pipeline Adaptation

pi-vcc's core pipeline operates on pi's `Message` type. BrowserOS uses AI SDK's `ModelMessage`. The algorithmic logic is identical — only the input normalization changes.

### New files

```
apps/server/src/agent/compaction/vcc/
  normalize.ts      — adapted from pi-vcc, accepts ModelMessage[]
  filter-noise.ts   — vendored from pi-vcc (format-agnostic)
  build-sections.ts — vendored from pi-vcc (format-agnostic)
  format.ts         — vendored from pi-vcc, accepts VccOverrides
  extract/
    goals.ts        — vendored from pi-vcc
    files.ts        — vendored from pi-vcc
    preferences.ts  — vendored from pi-vcc
    commits.ts      — vendored from pi-vcc
  brief.ts          — vendored from pi-vcc
  types.ts          — NormalizedBlock, SectionData, VccOverrides
```

### Adapter entry point

```ts
// compaction/vcc-adapter.ts
export function vccCompile(
  messages: ModelMessage[],
  existingSummary: string | null,
  overrides?: VccOverrides,
): string | null {
  const blocks = filterNoise(normalizeFromAiSdk(messages));
  const data = buildSections({ blocks });
  const fresh = formatSummary(data, overrides);

  if (!fresh) return null;

  // Merge with existing summary if present
  if (existingSummary) {
    return mergePrevious(stripRecallNote(existingSummary), fresh);
  }
  return fresh;
}
```

### normalizeFromAiSdk()

Maps AI SDK `ModelMessage` to pi-vcc's `NormalizedBlock[]`:

```ts
function normalizeFromAiSdk(messages: ModelMessage[]): NormalizedBlock[] {
  const blocks: NormalizedBlock[] = [];
  for (const msg of messages) {
    if (msg.role === "user") {
      blocks.push({ kind: "user", text: extractText(msg.content) });
    } else if (msg.role === "assistant") {
      const { text, toolCalls } = extractAssistant(msg.content);
      if (text) blocks.push({ kind: "assistant", text });
      for (const tc of toolCalls) {
        blocks.push({ kind: "tool_call", name: tc.name, args: tc.args });
      }
    } else if (msg.role === "tool") {
      for (const part of msg.content) {
        if (part.type === "tool-result") {
          blocks.push({
            kind: "tool_result",
            name: part.toolName ?? "unknown",
            text: toolResultOutputToText(part.output),
            isError: part.output.type === "error-text" || part.output.type === "error-json",
          });
        }
      }
    }
  }
  return blocks;
}
```

The `extractText()` and `extractAssistant()` helpers already exist in `compaction/prompt.ts` as `extractTextContent()` and `extractAssistantContent()`. Reuse them.

## Custom Prompt for Default Method

Modify `buildSummarizationPrompt()`:

```ts
export function buildSummarizationPrompt(
  existingSummary: string | null,
  customPrompt?: string | null,  // NEW parameter
): string {
  if (customPrompt) {
    // User provided their own prompt — use it instead of built-in format
    const base = existingSummary
      ? `Update the existing summary with new information.\n\n<previous_summary>\n${existingSummary}\n</previous_summary>`
      : `Summarize the following conversation transcript.`;
    return `${base}\n\n${customPrompt}`;
  }

  // Existing behavior unchanged
  if (existingSummary) {
    return `${UPDATE_PROMPT}\n\n<previous_summary>\n${existingSummary}\n</previous_summary>`;
  }
  return INITIAL_PROMPT;
}
```

## File Changes Summary

| File | Change |
|------|--------|
| `apps/server/src/agent/types.ts` | Add `CompactionStrategyConfig` interface + `compaction?` field to `ResolvedAgentConfig` |
| `apps/server/src/agent/compaction.ts` | Add `compactionConfig` param to `compactMessages()` and `createCompactionPrepareStep()`; route by method |
| `apps/server/src/agent/compaction/prompt.ts` | Add `customPrompt` param to `buildSummarizationPrompt()` |
| `apps/server/src/agent/compaction/vcc/` | NEW directory — vendored pi-vcc pipeline adapted for AI SDK types |
| `apps/server/src/agent/compaction/vcc-adapter.ts` | NEW — `vccCompile()`, `normalizeFromAiSdk()` |
| `apps/server/src/agent/ai-sdk-agent.ts` | Pass `config.resolvedConfig.compaction` to `createCompactionPrepareStep()` |
| `packages/shared/src/constants/config-schema.ts` | New config keys for compaction method, custom prompt |
| `apps/server/tests/agent/compaction.test.ts` | New tests for strategy routing, VCC path, custom prompt |

## The 4 Modes

| Config | Behavior | LLM Calls | Latency |
|--------|----------|-----------|---------|
| No `compaction` field | Default LLM summarization | 1-2 | ~2-10s |
| `method: "default"` + `customPrompt` | LLM with user's prompt | 1-2 | ~2-10s |
| `method: "vcc"` | Algorithmic, no LLM | 0 | 30-470ms |
| `method: "vcc"` + `vccConfig` | Algorithmic with custom caps | 0 | 30-470ms |

## Fallback Chain (unchanged)

Regardless of method, if compaction produces:
- Empty/null summary → sliding window fallback
- Summary larger than original → sliding window fallback
- Summarization timeout/abort → sliding window fallback

This chain is shared across both methods.

## Backwards Compatibility

- No `compaction` field → identical behavior to current code
- All existing config keys work unchanged
- All existing tests pass without modification
- VCC code is isolated in `compaction/vcc/` — not imported when method is "default"

## Testing

- **Unit:** `vccCompile()` with sample `ModelMessage[]` → produces structured summary
- **Unit:** `normalizeFromAiSdk()` correctly maps all AI SDK message types
- **Unit:** `buildSummarizationPrompt()` with and without `customPrompt`
- **Unit:** `compactMessages()` routes to correct method based on config
- **Integration:** Full `prepareStep` with VCC method → messages stay under threshold
- **Regression:** Existing `compaction.test.ts` passes without changes
