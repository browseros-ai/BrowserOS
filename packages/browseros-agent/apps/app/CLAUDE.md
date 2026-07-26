# BrowserOS App UI contributor ground rules

The app UI is a WXT React extension: side panel chat, app/settings pages, new tab, onboarding, background workers, and content scripts.

## Before you push

From the monorepo root:

```
bun run lint
bun run typecheck
bun run build:agent
```

For focused agent UI work:

```
cd apps/app && bun run typecheck
cd apps/app && bun run test
cd apps/app && bun run codegen
```

## Project shape

```
apps/app/
|- entrypoints/
|  |- sidepanel/     Chat UI
|  |- app/           Settings, AI providers, agents, MCP, usage
|  |- newtab/        BrowserOS new tab UI
|  |- onboarding/    First-run flow
|  |- background/    Extension background logic
|  `- *.content*     Page/content integrations
|- components/       Shared UI, including generated shadcn-style primitives
|- generated/graphql GraphQL codegen output
|- lib/              Auth, GraphQL, metrics, Sentry, BrowserOS clients, state
|- schema/           Default GraphQL schema input
`- wxt.config.ts     Manifest and WXT/Vite config
```

## WXT and entrypoints

- `wxt.config.ts` owns manifest shape, permissions, side panel/new tab/options wiring, extension ID, externally connectable hosts, and Vite plugins.
- `entrypoints/sidepanel/main.tsx` is the side panel entry.
- `entrypoints/app/main.tsx` is the extension app/settings entry.
- `entrypoints/newtab/` owns the new tab experience.
- `entrypoints/background/` owns background jobs and extension-level listeners.
- Content entrypoints live under `entrypoints/*.content*`; keep page integration logic there, not in shared UI components.

## UI conventions

- Folders are kebab-case. React component files are PascalCase. Hooks use a `use` prefix. Single-word utility/model files stay lowercase.
- Avoid `useCallback` and `useMemo` unless they solve a measured or obvious render problem.
- Build UI from the shadcn-style primitives in `components/ui/` and the AI Elements in `components/ai-elements/`. Both are generated — fallow skips its unused/leak checks for them (`.fallowrc.json`) — so treat them as generated output and don't hand-edit them for feature work.
- Feature UI lives in `components/<feature>/` or colocated with its entrypoint — keep `components/ui/` and `components/ai-elements/` for the generated primitives only.
- Capture runtime errors with Sentry, not `console.error`:

```
import { sentry } from '@/lib/sentry/sentry'

sentry.captureException(error, {
  extra: { message: 'Failed to fetch graph data from the server' },
})
```

## Server state and data fetching

All server state goes through **TanStack Query** (`@tanstack/react-query`) — don't fetch with `useEffect` + `useState`, and don't call the network from a component body. There are two lanes:

- **GraphQL (BrowserOS API)** — the default for app data. Colocated `graphql()` documents + the `lib/graphql/` helpers; see *GraphQL and codegen* below.
- **Local REST (agent harness, credits)** — endpoints on the dynamic agent server wrap a small `fetch` in `useQuery`/`useMutation`. Reference: `entrypoints/app/agents/useAgents.ts`, `lib/credits/useCredits.ts`.

Either lane: keep query keys in one place — derive GraphQL keys with `getQueryKeyFromDocument(Document)`, give REST hooks a module-level query-key const (e.g. `AGENT_QUERY_KEYS`) — and invalidate through that, never a hand-written string literal (`BrowserOsAiPane.tsx` invalidates via `getQueryKeyFromDocument(...)`). For instant-feeling mutations, do optimistic updates with `onMutate` -> `cancelQueries` + `getQueryData` + `setQueryData`, rolling back in `onError` — worked example: `useUpdateHarnessAgent` in `entrypoints/app/agents/useAgents.ts`.

## GraphQL and codegen

- Codegen input defaults to `schema/schema.graphql`; set `GRAPHQL_SCHEMA_PATH` when you need an external schema.
- Generated files live in `generated/graphql/`; do not hand-edit them.
- Put GraphQL documents in a local `graphql/` folder near the feature using them.
- Import documents with `graphql` from `@/generated/graphql/gql`.
- Use the existing helpers in `lib/graphql/`: `useGraphqlQuery`, `useGraphqlMutation`, `useGraphqlInfiniteQuery`, and `getQueryKeyFromDocument`.
- After adding or changing a document, run:

```
cd apps/app && bun run codegen
```

## Forms

Every form uses `react-hook-form` for state and submission plus a single `zod` schema for validation, bridged by `@hookform/resolvers/zod` — no `useState`-per-field. The UI is the shadcn `Form` set (`Form`, `FormField`, `FormItem`, `FormLabel`, `FormControl`, `FormMessage`) from `@/components/ui/form`. The schema is the source of truth: derive `FormValues` with `z.infer`, pass `zodResolver(schema)` to `useForm`, and surface per-field errors through `<FormMessage />` instead of rolling your own error state.

- Import `z` from `zod/v3`, not `zod`. The package ships zod 4, which exposes a `zod/v3` compatibility entry; every existing form standardizes on it, so match them.
- Reference: `entrypoints/app/connect-mcp/AddCustomMCPDialog.tsx` (also `ai-settings/NewProviderDialog.tsx`, `scheduled-tasks/NewScheduledTaskDialog.tsx`).

## Routing

Each page entrypoint owns a `react-router` v7 `HashRouter` with one central route table — `entrypoints/app/App.tsx` for the app/settings pages, `entrypoints/sidepanel/App.tsx` for the side panel. Add a page by registering a `<Route>` there and colocating the screen under `entrypoints/<area>/<feature>/`; express redirects and back-compat paths as `<Navigate replace>` entries in the same table.

## Dates and times

`dayjs` is the one date library (already a dependency) — use it for parsing, comparison, and formatting; prefer it over ad-hoc `Date`/`Intl` math, and don't add a second date lib. There is no central date module: keep a reusable formatter in the owning feature's helper file (e.g. `entrypoints/sidepanel/history/components/utils.ts`) rather than re-deriving the same bucketing inline.

## Module boundaries and formatting

- From the monorepo root, run `bun run fallow` before pushing. It flags unused files/exports, circular dependencies, and private-type leaks (`.fallowrc.json`); `generated/**` is ignored, and fallow skips its unused/leak checks for `components/ui/**` and `components/ai-elements/**`.
- Colocate a feature's pieces with the entrypoint that owns them — its `graphql/` documents, `*.helpers.ts`, and `*.test.ts` sit next to it. Only genuinely shared code goes in `lib/` or `components/`, imported via the `@/` alias.
- Biome owns formatting and import order — run `bun run lint:fix` rather than hand-formatting.
- Comments: default to none and explain *why*, not *what*; see `packages/browseros-agent/CLAUDE.md` for the full rule.

## Analytics

- Event constants live in `lib/constants/analyticsEvents.ts`.
- Event constants use `SCREAMING_SNAKE_CASE` ending in `_EVENT`.
- Add `/** @public */` above each exported event constant.
- Event values follow `<area>.<entity>.<action>` such as `ui.message.like` or `settings.managed_mcp.added`.
- Always call `track()` with an event constant; never pass raw event strings.

## Self-testing UI changes

Use the CDP inspector when changing extension UI. It can inspect extension pages that the agent tools cannot see.

Start the dev environment and read the randomized CDP port:

```
bun run dev:watch -- --new
export BROWSEROS_CDP_PORT=<port from output>
```

Useful inspector commands:

```
bun scripts/dev/inspect-ui.ts targets
bun scripts/dev/inspect-ui.ts open-sidepanel
bun scripts/dev/inspect-ui.ts snapshot sidepanel
bun scripts/dev/inspect-ui.ts screenshot sidepanel /tmp/panel.png
bun scripts/dev/inspect-ui.ts click sidepanel <backendDOMNodeId>
bun scripts/dev/inspect-ui.ts fill sidepanel <backendDOMNodeId> "search query"
bun scripts/dev/inspect-ui.ts press_key sidepanel Enter
bun scripts/dev/inspect-ui.ts eval sidepanel "document.title"
```

The normal loop is `snapshot -> click/fill/press_key -> screenshot`. Element IDs are the `[number]` values from the snapshot output.

## When in doubt, read a sibling

Most features are a vertical slice you can copy. The AI settings slice is the GraphQL template end to end: `entrypoints/app/ai-settings/graphql/aiSettingsDocument.ts` (documents) -> consumed via `lib/graphql` hooks in `ai-settings/BrowserOsAiPane.tsx` -> `ai-settings/NewProviderDialog.tsx` (the `zod` + shadcn `Form` dialog). For the REST lane plus optimistic mutations, `entrypoints/app/agents/useAgents.ts` is the worked example.
