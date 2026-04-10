# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Coding guidelines

- Write minimal code comments. Only add comments for non-obvious logic, complex algorithms, or critical warnings. Skip comments for self-explanatory code, obvious function names, and simple operations.

## File Naming Convention

| Type | Convention | Example |
|------|------------|---------|
| Folders | kebab-case | `ai-settings/`, `jtbd-popup/`, `llm-hub/` |
| React components (.tsx) | PascalCase | `AISettingsPage.tsx`, `SurveyHeader.tsx` |
| Hooks (.ts) | camelCase with `use` prefix | `useVoiceInput.ts`, `useMessageTree.ts` |
| Non-component files (.ts) | lowercase | `types.ts`, `models.ts`, `storage.ts` |

## Project Overview

**BrowserOS Agent Chrome Extension** - This project contains the official chrome extension for BrowserOS Agent, enabling users to interact with the core functionalities of BrowserOS.

## Bun Preferences

Default to using Bun instead of Node.js:

- Use `bun <file>` instead of `node <file>`
- Use `bun test` instead of `jest` or `vitest`
- Use `bun install` instead of `npm install`
- Use `bun run <script>` instead of `npm run <script>`
- Bun automatically loads .env (no dotenv needed)

## Project Structure

This project user wxt.dev as its framework for building chrome extension.

The chrome extension manifest is created via default wxt.dev setup along with some custom configuration provided via `wxt.config.ts` file

The key directories of the project are:
- `entrypoints/newtab`: Contains the code for the new tab page of the extension.
- `entrypoints/popup`: Contains the code for the popup that appears when the extension icon is clicked.
- `entrypoints/onboarding`: Contains the onboarding flow for new users which is triggered on first install.

## React Coding patterns

- Avoid using useCallback and useMemo as much as possible - only add them if their presence is absolutely necessary
- When writing a graphql document, create a /graphql directory under the current directory where the file is present and create a file to contain the document.
  - For example: if you want to create grapqhl queries in @apps/agent/entrypoints/sidepanel/history/ChatHistory.tsx then write the graphql document in @apps/agent/entrypoints/sidepanel/history/graphql/chatHistoryDocument.ts 
- Shadcn UI is setup in this project and always use shadcn components for the UI
- When need to record errors, do not use console.error -> instead use the sentry service to capture errors:
```ts
import { sentry } from '@/lib/sentry/sentry'

sentry.captureException(error, {
  extra: {
    message: 'Failed to fetch graph data from the server',
    codeId: workflow.codeId,
  },
})
```

## GraphQL Client

- The Graphql main schema file is in `@apps/agent/generated/graphql/schema.graphql` - this is the source of truth for constructing all graphql queries

- The frontend uses React Query with `graphql-codegen` to interact with the backend GraphQL API. The types are generated and stored in `@apps/agent/generated/graphql`

- When working with React Query and GraphQL, some important utilities are already created to make the interaction simpler:
  - `@apps/agent/lib/graphql/useGraphqlInfiniteQuery.ts`
  - `@apps/agent/lib/graphql/useGraphqlMutation.ts`
  - `@apps/agent/lib/graphql/useGraphqlQuery.ts`
  - `@apps/agent/lib/graphql/getQueryKeyFromDocument.ts`

This is how a standard GraphQL query and mutation looks like:

```ts
import { graphql } from "~/graphql/gql";
import { useGraphqlQuery } from "@/lib/graphql/useGraphqlQuery";
import { useGraphqlMutation } from "@/lib/graphql/useGraphqlMutation";
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { getQueryKeyFromDocument } from "@/modules/graphql/getQueryKeyFromDocument";
import { useQueryClient } from "@tanstack/react-query";

export const GetProfileByUserIdDocument = graphql(`
  query GetProfileByUserId($userId: String!) {
    profileByUserId(userId: $userId) {
      id
      rowId
      name
      userId
      meta
      profilePictureUrl
      linkedInUrl
      updatedAt
      createdAt
      deletedAt
    }
  }
`);

const UpdateProfileIndustryDocument = graphql(`
  mutation UpdateProfileIndustry($userId: String!, $meta: JSON) {
    updateProfileByUserId(input: { userId: $userId, patch: { meta: $meta } }) {
      profile {
        id
        rowId
        meta
      }
    }
  }
`);

  const { sessionInfo } = useSessionInfo()

  const userId = sessionInfo.user?.id

const queryClient = useQueryClient();

const { data: profileData } = useGraphqlQuery(
  GetProfileByUserIdDocument,
  {
    userId,
  },
  {
    enabled: !!userId,
  },
);

const updateProfileMutation = useGraphqlMutation(
  UpdateProfileIndustryDocument,
  {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: [getQueryKeyFromDocument(GetProfileByUserIdDocument)],
      });
    },
  },
);
```

To run codegen to generate graphql code after creating a query, you should run codegen using the command (since .env.development is necessary for codegen):
```sh
bun --env-file=.env.development run codegen
```

## Analytics & Event Tracking

All user-facing events are tracked using a centralized pattern:

- **Event constants** are defined in `lib/constants/analyticsEvents.ts`
- **Tracking** is done via the `track()` utility from `lib/metrics/track.ts`

**Event constant naming:**
- Use `SCREAMING_SNAKE_CASE` ending with `_EVENT`
- Add `/** @public */` JSDoc tag above each constant

**Event value naming** follows the pattern `<area>.<entity>.<action>`:
- `ui.*` - sidepanel/chat interactions (e.g. `ui.message.like`, `ui.conversation.reset`)
- `settings.*` - settings page actions (e.g. `settings.scheduled_task.created`, `settings.managed_mcp.added`)
- `newtab.*` - new tab page actions (e.g. `newtab.opened`, `newtab.ai.triggered`)
- `sidepanel.*` - sidepanel-specific actions (e.g. `sidepanel.ai.triggered`)

**Usage:**
```ts
import { MY_EVENT } from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'

// Without properties
track(MY_EVENT)

// With properties
track(MY_EVENT, {
  mode,
  provider_type: selectedLlmProvider?.type,
})
```

Always use event constants from `analyticsEvents.ts` — never pass raw string event names to `track()`.
