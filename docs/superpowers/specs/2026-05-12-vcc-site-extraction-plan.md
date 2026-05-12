# Site-Aware VCC Extraction — Implementation Plan

## Goal

Add site+tool extraction to BrowserOS VCC compaction so the compressed summary retains:
1. **Which sites were visited** (URLs)
2. **Tool usage per site** (`toolName*times`)
3. **Timeline view** (sequential site visits)

## Current State

The vendored pi-vcc extractors are **coding-oriented** (files, commits, bash). BrowserOS tools are **browser-oriented** (navigate, click, snapshot, fill, scroll). The current `extractPath()` and `toolOneLiner()` in `brief.ts` only know about `path`, `file_path`, `command` — they never extract URLs or page IDs.

## BrowserOS Tool Taxonomy

### Navigation (carries URL)
- `navigate_page` → args: `{ url, page }`
- `new_page` → args: `{ url }`
- `new_hidden_page` → args: `{ url }`

### Page interaction (carries page ID)
- `click`, `click_at`, `fill`, `type_at`, `hover`, `hover_at`, `scroll`, `drag`, `drag_at`, `focus`, `clear`, `press_key`, `handle_dialog` → args: `{ page, ... }`

### Observation (carries page ID)
- `take_snapshot`, `take_enhanced_snapshot`, `take_screenshot`, `get_page_content`, `get_page_links`, `get_dom`, `search_dom`, `evaluate_script`, `get_console_logs` → args: `{ page, ... }`

### Tab management
- `list_pages`, `get_active_page`, `close_page`, `move_page`, `show_page`

### Not site-related
- `list_windows`, `create_window`, `close_window`, `activate_window`
- `save_pdf`, `save_screenshot`, `download_file`
- bookmarks, history, memory tools

## Design

### 1. New type: `SiteActivity`

```ts
interface SiteVisit {
  url: string           // full URL
  domain: string        // hostname for grouping
  order: number         // sequential visit index (timeline)
  sourceIndex: number   // original message index
  tools: Map<string, number>  // toolName → count
}

interface SiteTimeline {
  visits: SiteVisit[]   // ordered by sourceIndex
}
```

### 2. New file: `extract/activities.ts`

Extracts site visits from NormalizedBlock stream:

```
navigate_page({ url: "https://example.com" }) → creates SiteVisit
click({ page: 5 }) → associates with current active site
take_snapshot({ page: 5 }) → increments tool count for active site
navigate_page({ url: "https://other.com" }) → creates new SiteVisit
```

Logic:
1. Track `currentUrl: string | null` and `currentPageId: number | null`
2. On `navigate_page`/`new_page`: create new SiteVisit, set current
3. On any tool with `page` arg: if page matches current page, increment tool count
4. On `navigate_page` with different URL: new visit
5. Group consecutive visits to same domain (same URL counts once, but tool counts accumulate)

### 3. Format: `[Site Activity]` section in summary

```
[Site Activity]
1. example.com — navigate_page*3, click*5, fill*2, take_snapshot*4
2. github.com/user/repo — navigate_page*2, click*3, get_page_content*1
3. docs.google.com — navigate_page*1, fill*8, click*12, evaluate_script*2
```

Per-site line: `<order>. <domain> — <tool1>*<count>, <tool2>*<count>, ...`

Rules:
- Sort tools by count descending
- Only show domain (not full URL) — keeps it compact
- Full URL used only when domain is ambiguous (e.g., different paths on same domain → group together)
- Max 10 sites (truncate oldest if exceeded)
- If same domain visited multiple times, show combined tool counts with total visits in parens: `example.com (3 visits) — ...`

### 4. Timeline: `[Timeline]` section

```
[Timeline]
#1 → example.com          [tools: 14]
#2 → github.com/user/repo [tools: 6]
#3 → docs.google.com      [tools: 23]
  ↳ back to example.com   [tools: 5]
```

Shows the flow of site navigation with tool counts per visit.

### 5. Changes to existing files

**`types.ts`**: Add `SiteVisit[]` to `SectionData`
**`build-sections.ts`**: Call `extractActivities()`, add to return
**`format.ts`**: Format site activity + timeline sections
**`tool-args.ts`**: Add `extractUrl()` helper for URL extraction from BrowserOS tool args

### 6. Tests

New test file: `tests/agent/compaction/vcc-activities.test.ts`

Test cases:
- Single site, multiple tools
- Multiple sites, sequential visits
- Return visit to same domain
- No navigation tools (no site activity)
- Mixed browser + non-browser tools
- Same tool many times (count accuracy)
- Max site cap (truncation)
- Timeline ordering

## Files to create/modify

| File | Action | Purpose |
|------|--------|---------|
| `compaction/vcc/extract/activities.ts` | NEW | Site extraction logic |
| `compaction/vcc/types.ts` | MODIFY | Add SiteVisit type to SectionData |
| `compaction/vcc/tool-args.ts` | MODIFY | Add extractUrl helper |
| `compaction/vcc/build-sections.ts` | MODIFY | Call extractActivities |
| `compaction/vcc/format.ts` | MODIFY | Format site activity + timeline |
| `tests/agent/compaction/vcc-activities.test.ts` | NEW | Tests |

## Constraints

- Do NOT touch normalize.ts or brief.ts (they work fine)
- Keep format compact — `<domain> — tool*N, tool*N`
- Tool counts only, no args listing
- Timeline shows navigation flow, not every action
- Max 10 sites, truncate oldest
