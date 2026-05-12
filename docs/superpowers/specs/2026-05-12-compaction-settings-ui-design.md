# Compaction Settings UI

**Date:** 2026-05-12
**Status:** Draft
**Scope:** Add a Compaction settings page under Settings sidebar, exposing the compaction strategy config to end users.

**Skills invoked:** brainstorming

## Context

The VCC compaction feature (spec: `2026-05-12-compaction-strategy-switching-design.md`) is implemented server-side but has no UI. Users must manually edit `config.json` to change compaction method. This spec adds a settings page at `/settings/compaction`.

## Architecture

### Pattern

Follow the existing settings page pattern exactly:

```
SettingsSidebarLayout → Route → CompactionPage
SettingsSidebar → NavItem → "Compaction" → /settings/compaction
```

Each settings page has:
1. A header card with icon, title, description
2. Content sections with form controls
3. Save/Reset buttons
4. Pending restart banner when changes need restart

### Data Flow

```
CompactionPage → useCompactionConfig() hook
  → GET /api/compaction  (read current config)
  → PUT /api/compaction  (save new config)
  → DELETE /api/compaction (reset to defaults)
```

The compaction config is **server-startup-level** (from `config.json`), NOT the same as advanced config (which is `configStore` with numeric key-values). Compaction has an enum method + optional nested object, so it needs its own API route.

### Comparison with Advanced Config

| Aspect | Advanced Config | Compaction Config |
|--------|----------------|-------------------|
| Store | `configStore` (shared) | `config.json` via `ServerConfig` |
| Types | `Record<string, number>` | `{ method: enum, customPrompt?: string, vccConfig?: {...} }` |
| API | `GET/PUT/DELETE /config` | `GET/PUT/DELETE /compaction` |
| Validation | Min/max numeric | Zod schema from `CompactionStrategySchema` |
| Restart | Required | Required |

## File Changes

### New Files

#### 1. `apps/server/src/api/routes/compaction.ts`

New API route for compaction config CRUD.

```
GET  /  → Read current compaction from config file
PUT  /  → Write compaction to config file (merges with existing config.json)
DELETE / → Remove compaction from config file (revert to default)
```

**GET response:**
```ts
interface CompactionConfigResponse {
  active: {
    method: 'default' | 'vcc'
    customPrompt?: string
    vccConfig?: {
      maxTranscriptLines?: number
      maxGoalLines?: number
      maxFileEntries?: number
      maxCommitEntries?: number
      maxPreferenceLines?: number
      maxOutstandingLines?: number
    }
  } | null
  defaults: {
    method: 'default'
  }
}
```

**PUT body:**
```ts
{
  method: 'default' | 'vcc'
  customPrompt?: string
  vccConfig?: { ... }
}
```

Implementation: Read `config.json` via `getServerConfigPath()`, parse, update `compaction` field, write back. Use zod schema `CompactionStrategySchema` for validation.

#### 2. `apps/agent/entrypoints/app/compaction-settings/CompactionSettingsPage.tsx`

Settings page component. Structure matches `AdvancedConfigPage.tsx`:

```
<div className="space-y-6 p-6">
  <!-- Header card (icon + title + description) -->
  <!-- Method selector section -->
  <!-- Conditional sections based on method -->
  <!-- Save / Reset buttons -->
</div>
```

#### 3. `apps/agent/entrypoints/app/compaction-settings/compaction-queries.ts`

React Query hook matching `config-queries.ts` pattern:

```ts
export function useCompactionConfig() {
  // GET /compaction
  // PUT /compaction
  // DELETE /compaction
  return { config, isLoading, error, saveConfig, resetConfig, isSaving, isResetting }
}
```

#### 4. `apps/agent/entrypoints/app/compaction-settings/MethodSelector.tsx`

Radio group for method selection (default / vcc).

#### 5. `apps/agent/entrypoints/app/compaction-settings/VccConfigSection.tsx`

Collapsible section with number inputs for VCC overrides. Only visible when method = 'vcc'.

### Modified Files

#### 6. `apps/agent/components/sidebar/SettingsSidebar.tsx`

Add nav item under "Other" section:

```ts
{ name: 'Compaction', to: '/settings/compaction', icon: Layers }
```

Icon: `Layers` from lucide-react (represents compression/layering).

#### 7. `apps/agent/entrypoints/app/App.tsx`

Add route inside `<Route path="settings">`:

```tsx
<Route path="compaction" element={<CompactionSettingsPage />} />
```

#### 8. `apps/server/src/api/server.ts`

Mount compaction routes:

```ts
.route('/compaction', createCompactionRoutes())
```

## UI Layout

```
┌──────────────────────────────────────────────┐
│  🗜️  Compaction                               │
│  Configure how conversation history is        │
│  compressed when the context window fills up. │
│  Changes require restarting BrowserOS.        │
└──────────────────────────────────────────────┘

Method
─────────────────────────────────────────────────
  ○ Default (LLM Summarization)
    Uses the AI model to summarize conversation history.
    Produces a structured summary with Goal, Progress, Next Steps.

    Custom Prompt (optional)
    ┌─────────────────────────────────────────────┐
    │ Override the default summarization prompt... │
    │                                             │
    └─────────────────────────────────────────────┘

  ○ VCC (Algorithmic)
    Structured extraction without LLM calls.
    Faster (30-470ms vs 1-2s), zero token cost.

    ┌─ VCC Configuration ────────────────────────┐
    │ Max Transcript Lines     [120     ▾]       │
    │ Max Goal Lines           [8       ▾]       │
    │ Max File Entries         [10      ▾]       │
    │ Max Commit Entries       [8       ▾]       │
    │ Max Preference Lines     [15      ▾]       │
    │ Max Outstanding Lines    [10      ▾]       │
    └─────────────────────────────────────────────┘

─────────────────────────────────────────────────
[Reset to Defaults]              [Save Changes]
```

## Component Details

### MethodSelector

- Two radio buttons styled as cards (matching existing pattern)
- Each option has a title and description
- Selecting "Default" shows optional customPrompt textarea
- Selecting "VCC" shows VccConfigSection

### VccConfigSection

- Collapsible group with 6 number inputs
- Each input has label, current value, and unit label ("lines", "entries")
- Uses `ConfigField` component from `advanced-config/ConfigField.tsx` if compatible, otherwise custom

### Header Card

Matches AdvancedConfigPage exactly:
```tsx
<div className="flex items-start gap-4 rounded-xl border p-5">
  <div className="rounded-lg bg-muted p-2">
    <Layers className="size-6" />
  </div>
  <div>
    <h2 className="font-semibold text-lg">Compaction</h2>
    <p className="text-muted-foreground text-sm">
      Configure how conversation history is compressed...
    </p>
  </div>
</div>
```

## API Route Implementation

The compaction config lives in `config.json` (loaded at startup via `loadServerConfig`). The API route needs to:

1. **GET**: Read `config.json`, extract `compaction` field, return with defaults
2. **PUT**: Read `config.json`, validate new compaction value via `CompactionStrategySchema`, merge, write back
3. **DELETE**: Read `config.json`, remove `compaction` field, write back

Uses `getServerConfigPath()` from `lib/browseros-dir.ts` to find the config file.

## Data Shapes

### CompactionConfigResponse (GET)

```ts
{
  active: CompactionStrategyConfig | null  // null = no compaction override
  defaults: { method: 'default' }         // always default method
}
```

### CompactionConfigSaveRequest (PUT)

```ts
{
  method: 'default' | 'vcc'
  customPrompt?: string
  vccConfig?: {
    maxTranscriptLines?: number
    maxGoalLines?: number
    maxFileEntries?: number
    maxCommitEntries?: number
    maxPreferenceLines?: number
    maxOutstandingLines?: number
  }
}
```

### CompactionConfigSaveResponse (PUT/DELETE)

```ts
{
  ok: boolean
  saved?: CompactionStrategyConfig
  errors?: Array<{ key: string; message: string }>
}
```
