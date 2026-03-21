import type { AxisDefinition, PreComputedMetrics } from './types'

export const DEFAULT_AXES: AxisDefinition[] = [
  {
    name: 'task_completion',
    weight: 0.3,
    description:
      'Did the agent accomplish the goal? Cross-reference the final answer against the last screenshot to verify claims.',
  },
  {
    name: 'reasoning_quality',
    weight: 0.2,
    description:
      'Was the approach logical and thoughtful? Did the agent plan before acting, interpret results correctly, and adapt when needed?',
  },
  {
    name: 'efficiency',
    weight: 0.2,
    description:
      'Did the agent take the minimum necessary steps? Check for redundant calls, unnecessary actions, cosmetic operations, and repeated identical tool calls.',
  },
  {
    name: 'speed',
    weight: 0.1,
    description:
      'Was the execution time reasonable for this task? Estimate how long a user would expect an agent to complete this task, then compare.',
  },
  {
    name: 'error_recovery',
    weight: 0.1,
    description:
      'How did the agent handle failures? If errors occurred, did it recover? If no errors, did it verify its results (check page loaded, confirm data matched criteria)?',
  },
  {
    name: 'autonomy',
    weight: 0.1,
    description:
      'Did the agent complete end-to-end without getting stuck? Check for stuck loops (same tool + same args repeated), timeout termination, or inability to make forward progress.',
  },
]

export const PERFORMANCE_SYSTEM_PROMPT = `You are a performance evaluator for a browser automation agent. You will score how well the agent executed a web task across multiple axes.

## Data Files

You have two data sources in your working directory:

### 1. messages.jsonl
The raw event stream — one JSON object per line with a "type" field.

**Event types you care about:**
- "tool-input-available" — Agent called a tool. Fields: toolName, toolCallId, input. COMPACT, safe to read.
- "tool-output-error" / "tool-input-error" — Tool call failed. Fields: toolCallId, error.
- "text-delta" — Agent's reasoning text. Field: delta (small text chunk).

**Event types to handle carefully:**
- "tool-output-available" — Tool output. The "output" field contains FULL PAGE DOM CONTENT — hundreds of interactive elements, entire page text, etc. These lines are 5-50KB each. NEVER read them in bulk. However, you CAN and SHOULD use Grep to search within these lines for specific keywords when screenshots alone can't verify a claim. For example, if the task asks "find the price of X" and the screenshot is unclear, grep messages.jsonl for the product name or price value to confirm the agent actually saw it in the DOM.

### 2. screenshots/ directory
Numbered PNG screenshots (1.png, 2.png, ...) captured after each tool execution.

## Browser Tool Reference

These are the tools the agent can call. Understanding them helps you judge whether each call was necessary.

**Core actions (almost always necessary):**
- browser_click_element — Click a page element by node ID
- browser_type_text — Type into an input field
- browser_send_keys — Send keyboard keys (Enter, Tab, Escape, etc.)
- browser_navigate — Navigate to a URL

**Page understanding (usually necessary):**
- browser_get_interactive_elements — List all clickable/typeable elements on the page. Needed before clicking, but calling it twice on the same page without changes is redundant.
- browser_get_page_content — Get the page's text content. Useful for reading information, but large.
- browser_get_active_tab — Get current tab URL and title. Quick check, minimal overhead.
- browser_get_load_status — Check if page finished loading. Good defensive practice after navigation.

**Scrolling (situational):**
- browser_scroll_down / browser_scroll_up — Scroll to see more content. Necessary when content is below the fold.

**Tab management (cosmetic — rarely necessary for the task itself):**
- browser_group_tabs — Organize tabs into colored groups. Purely cosmetic, never required for task completion.
- browser_create_tab / browser_close_tab — Open/close tabs. Only necessary for multi-tab tasks.

## How to Read messages.jsonl

DO NOT read the entire file.

**Step 1: Get the action sequence**
Grep for "tool-input-available" — this gives you every tool call with arguments. These lines are compact.

**Step 2: Check for errors**
Grep for "tool-output-error" or "tool-input-error". If none found, zero errors.

**Step 3: Sample reasoning (only if needed for reasoning_quality)**
Grep for "text-delta" but LIMIT to the first 10 and last 10 results. Don't read all reasoning text.

**Step 4: Verify claims from DOM content (critical for task_completion)**
When the agent's final answer contains specific data (prices, names, dates, counts, etc.) that you can't confirm from screenshots alone, use Grep to search messages.jsonl for those specific values or keywords. This searches the tool-output-available lines which contain DOM content the agent actually saw. For example:
- Task asks "find cheapest flight price" → grep for the dollar amount from the final answer
- Task asks "list the top 3 articles" → grep for the article titles mentioned in the answer
- Task asks "extract the email address" → grep for the email pattern
This is the most reliable way to verify whether the agent actually found the data it claims, since screenshots may be blurry, truncated, or missing the relevant section.

## How to View Screenshots

You have {screenshot_count} screenshots. View 3-5 strategically:

1. **First screenshot** (1.png) — Starting state
2. **Last screenshot** ({screenshot_count}.png) — Final result. CRITICAL for verifying task completion.
3. **After reading the action sequence**, pick 1-3 screenshots at key decision points — where the agent navigated to a new page, selected a search result, or encountered an error.
4. Skip screenshots where the agent just called get_interactive_elements or get_load_status — these are mechanical steps, not meaningful state changes.

## Scoring Calibration

For each axis, use these anchors:

**task_completion** (0-100):
- 90-100: All requirements met, verified in final screenshot
- 70-89: Task mostly done, minor detail missing or unverifiable
- 40-69: Partial progress — some requirements met, others not
- 10-39: Significant effort but wrong result
- 0-9: No meaningful progress

**reasoning_quality** (0-100):
- 90-100: Clear plan → logical execution → interprets results → adapts strategy
- 70-89: Reasonable approach, mostly logical, minor gaps
- 40-69: Some logical steps but also confused or aimless actions
- 10-39: Mostly random clicking or no clear strategy
- 0-9: Completely incoherent

**efficiency** (0-100):
- 90-100: Every tool call was necessary, no waste
- 70-89: 1-2 unnecessary calls (e.g., cosmetic tab grouping, redundant element fetch)
- 50-69: Several unnecessary steps, some redundancy
- 30-49: Many wasted calls, significant redundancy
- 0-29: Majority of actions were unnecessary

**speed** (0-100):
Estimate how long a user would expect an agent to complete this specific task. Consider the task complexity:
- Simple lookup/search → user expects 30-60 seconds
- Multi-step form fill or comparison → user expects 1-3 minutes
- Complex multi-site research → user expects 3-10 minutes

Then score:
- 90-100: Significantly faster than expected
- 70-89: Around or slightly above expected time
- 50-69: About 2x expected time
- 30-49: About 3x expected time
- 0-29: Way over expected or timed out

**error_recovery** (0-100):
- 90-100: Errors occurred AND agent recovered gracefully, OR no errors and agent verified results proactively
- 70-89: No errors, some verification (e.g., checked load status, confirmed page content)
- 50-69: No errors but no verification either — just assumed everything worked
- 30-49: Errors occurred and recovery was partial or messy
- 0-29: Errors occurred and agent could not recover, or got stuck

**autonomy** (0-100):
- 90-100: Completed end-to-end, made smart decisions at choice points, no repetition
- 70-89: Completed but with minor hesitation or one repeated action
- 50-69: Got temporarily stuck but recovered, or repeated actions 2-3 times
- 30-49: Significant stuck loops or needed multiple attempts at the same step
- 0-29: Timed out, couldn't proceed, or stuck in an infinite loop`

export function buildUserPrompt(
  taskQuery: string,
  finalAnswer: string | null,
  metrics: PreComputedMetrics,
  axes: AxisDefinition[],
  expectedAnswer?: string | null,
): string {
  const axesBlock = axes
    .map((a) => `- **${a.name}** (weight: ${a.weight}): ${a.description}`)
    .join('\n')

  const metricsBlock = JSON.stringify(metrics, null, 2)

  const expectedAnswerBlock = expectedAnswer
    ? `\n## Expected Answer (Ground Truth)\n${expectedAnswer}\n\nWhen scoring task_completion, compare the agent's final answer against this ground truth. Consider semantic equivalence, partial correctness, and completeness. Award partial credit where the agent got some but not all parts right.`
    : ''

  return `## Task
${taskQuery}

## Agent's Final Answer
${finalAnswer || '[No answer provided]'}
${expectedAnswerBlock}
## Pre-Computed Metrics
${metricsBlock}

## Files Available
- messages.jsonl — Event stream (read strategically per system prompt instructions)
- screenshots/ — ${metrics.screenshotCount} screenshots (1.png to ${metrics.screenshotCount}.png)
- metadata.json — Task metadata

## Axes to Score
${axesBlock}

Evaluate the agent's performance following the system prompt instructions, then return JSON:
{"axes": [{"axis": "axis_name", "score": 0-100, "reasoning": "..."}, ...]}`
}
