import { createQuery } from 'react-query-kit'
import type { RunStatus } from '@/lib/status'
import type { RunHarness } from '@/modules/api/runs.hooks'

export type ActionVerb =
  | 'navigate'
  | 'read'
  | 'click'
  | 'type'
  | 'attach'
  | 'submit'

export interface ActionLogItem {
  id: string
  kind: 'action'
  verb: ActionVerb
  /** CSS-selector-ish node identifier, rendered in a small mono chip. */
  node: string
  /** Optional one-liner shown under the verb in non-compact mode. */
  text?: string
}

export interface ResolvedLogItem {
  id: string
  kind: 'resolved'
  /** What the user decided. */
  res: 'once' | 'always' | 'blocked' | 'handed-back'
  domain: string
  /** Human-readable action name (e.g. "Submit"). */
  action: string
}

export interface DoneLogItem {
  id: string
  kind: 'done'
  text: string
}

export type LogItem = ActionLogItem | ResolvedLogItem | DoneLogItem

export interface PendingApproval {
  kind: 'submit' | 'navigate' | 'payment' | 'delete' | 'upload'
  title: string
  detail: string
  domain: string
  /** Scope sentence shown above the action buttons. */
  scope: string
}

export interface PendingHandoff {
  /** Banner headline. */
  title: string
  /** One-liner under the title. */
  note: string
  /** Full detail inside the in-window challenge card. */
  detail: string
  domain: string
}

export interface PendingBlock {
  title: string
  detail: string
  reason: string
}

export interface RunStats {
  /** Output tokens consumed by the run so far. */
  tokens: string
  /** Reference baseline (e.g. comparison run) shown struck-through. */
  tokensRef?: string
  /** Action steps completed so far. */
  steps: string
}

export interface RunDetail {
  id: string
  agentId: string
  agentLabel: string
  harness: RunHarness
  status: RunStatus
  task: string
  /** The site the agent is currently on, used for the viewport stub. */
  site: string
  /** Wall-clock elapsed since the run started, e.g. "3m 41s". */
  elapsed: string
  /** Past actions + resolved tags + the optional done summary, in order. */
  log: LogItem[]
  /** The action the agent is currently working on, if any. */
  current?: ActionLogItem
  /** Approval card pinned in the activity panel when set. */
  pending?: PendingApproval
  /** Handoff overlay shown on the browser viewport when set. */
  handoff?: PendingHandoff
  /** Persistent blocked notice pinned below the run header when set. */
  blocked?: PendingBlock
  stats: RunStats
  /** Text shown in the bottom-center WorkingPill while the run is live. */
  liveLine: string
}

/**
 * Mock run-detail fixtures keyed by run id. The wire shape matches
 * what the eventual `/runs/:id` hono route + `/sse/runs/:id` event
 * stream will produce, so the only thing this hook needs when those
 * arrive is a fetcher swap plus a subscription effect.
 */
const FIXTURES: Record<string, RunDetail> = {
  'cld-concur': {
    id: 'cld-concur',
    agentId: 'cld-concur',
    agentLabel: 'Cowork . File expenses',
    harness: 'Claude Code',
    status: 'needs-ok',
    task: 'See my May invoices and file expenses on SAP Concur',
    site: 'concur.com',
    elapsed: '3m 41s',
    log: [
      {
        id: 'a1',
        kind: 'action',
        verb: 'navigate',
        node: 'concur.com/expense/new',
        text: 'Opened the New Report page from the expense dashboard.',
      },
      {
        id: 'a2',
        kind: 'action',
        verb: 'type',
        node: '#report-name',
        text: 'Set name to "May 2026 · Engineering".',
      },
      {
        id: 'r1',
        kind: 'resolved',
        res: 'once',
        domain: 'concur.com',
        action: 'attach receipts',
      },
      {
        id: 'a3',
        kind: 'action',
        verb: 'attach',
        node: '.line[2] receipt',
        text: 'Attached Figma invoice PDF to line 2.',
      },
    ],
    current: {
      id: 'a4',
      kind: 'action',
      verb: 'type',
      node: '.line[3] vendor',
      text: 'Filling in the Linear vendor row…',
    },
    pending: {
      kind: 'submit',
      title: 'Submit the May expense report for approval?',
      detail: '4 lines · $1,284.50 · routed to Dana R.',
      domain: 'concur.com',
      scope:
        'This permission applies to concur.com only. If the agent navigates elsewhere, it asks again.',
    },
    stats: {
      tokens: '12.4k',
      tokensRef: '14.1k',
      steps: '17',
    },
    liveLine: 'Filling 4 expense lines',
  },
  'cld-li': {
    id: 'cld-li',
    agentId: 'cld-li',
    agentLabel: 'Cowork . LinkedIn posts',
    harness: 'Claude Code',
    status: 'running',
    task: 'Draft and queue 3 LinkedIn posts about the launch',
    site: 'linkedin.com',
    elapsed: '5m 18s',
    log: [
      {
        id: 'a1',
        kind: 'action',
        verb: 'navigate',
        node: 'linkedin.com/feed',
        text: 'Opened the LinkedIn home feed.',
      },
      {
        id: 'a2',
        kind: 'action',
        verb: 'click',
        node: 'header.start-a-post',
        text: 'Opened the composer for the first post.',
      },
      {
        id: 'a3',
        kind: 'action',
        verb: 'type',
        node: '.composer body',
        text: 'Typed the first post about the BrowserOS launch.',
      },
      {
        id: 'a4',
        kind: 'action',
        verb: 'click',
        node: '.composer schedule',
        text: 'Scheduled the first post for 9am tomorrow.',
      },
    ],
    current: {
      id: 'a5',
      kind: 'action',
      verb: 'type',
      node: '.composer body',
      text: 'Typing the second post in the composer…',
    },
    stats: {
      tokens: '9.1k',
      steps: '14',
    },
    liveLine: 'Typing the 2nd post in the composer',
  },
  'cdx-sheet': {
    id: 'cdx-sheet',
    agentId: 'cdx-sheet',
    agentLabel: 'Codex . Pricing research',
    harness: 'Codex',
    status: 'running',
    task: 'Compile competitor pricing into a Google Sheet',
    site: 'docs.google.com',
    elapsed: '12m 04s',
    log: [
      {
        id: 'a1',
        kind: 'action',
        verb: 'navigate',
        node: 'docs.google.com/spreadsheets/new',
        text: 'Created a new spreadsheet for the pricing comparison.',
      },
      {
        id: 'a2',
        kind: 'action',
        verb: 'type',
        node: 'row[1]',
        text: 'Set the header row: Competitor / Plan / Price / Notes.',
      },
      {
        id: 'a3',
        kind: 'action',
        verb: 'navigate',
        node: 'competitor sites · 4 tabs',
        text: 'Pulled pricing pages from 4 competitor sites in sibling tabs.',
      },
    ],
    current: {
      id: 'a4',
      kind: 'action',
      verb: 'type',
      node: 'row[9]',
      text: 'Pasting row 9 of 12 into the sheet…',
    },
    stats: {
      tokens: '21.3k',
      steps: '36',
    },
    liveLine: 'Pasting row 9 of 12 into the sheet',
  },
}

const FALLBACK: RunDetail = {
  id: 'unknown',
  agentId: 'unknown',
  agentLabel: 'Unknown run',
  harness: 'Codex',
  status: 'idle',
  task: 'No run with this id is being tracked.',
  site: 'about:blank',
  elapsed: '0m 00s',
  log: [],
  stats: { tokens: '0', steps: '0' },
  liveLine: 'Waiting for the agent to start',
}

interface UseRunVariables {
  runId: string
}

export const useRun = createQuery<RunDetail, UseRunVariables>({
  queryKey: ['run'],
  fetcher: ({ runId }) =>
    new Promise((resolve) =>
      setTimeout(() => resolve(FIXTURES[runId] ?? FALLBACK), 60),
    ),
})
