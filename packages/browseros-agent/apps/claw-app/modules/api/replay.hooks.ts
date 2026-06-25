import { createQuery } from 'react-query-kit'
import type { RunStatus } from '@/lib/status'
import type { RunHarness } from '@/modules/api/runs.hooks'

export type ReplayVerb =
  | 'navigate'
  | 'read'
  | 'click'
  | 'type'
  | 'attach'
  | 'submit'
  | 'done'

export type ReplayKind = 'action' | 'approval' | 'block' | 'done'

export interface ReplayFrame {
  /** Seconds into the session. */
  t: number
  kind: ReplayKind
  verb: ReplayVerb
  /** Short node label, e.g. "Create New Report". */
  node: string
  /** Caption sentence rendered both in the viewport overlay and the timeline row. */
  caption: string
  /** Optional badge shown on the timeline row ("Allowed once", "Blocked"). */
  note?: string
}

export interface ReplayDetail {
  id: string
  agentLabel: string
  /** One-line task description shown in the top bar. */
  taskTitle: string
  harness: RunHarness
  /** Final run status, used to colour the header pill. */
  status: RunStatus
  /** Originating site host for the browser-chrome stub. */
  site: string
  /** When the run happened, e.g. "Jun 1, 2026". */
  startedAt: string
  /** Wall-clock duration as displayed in the stat strip. */
  duration: string
  tokens: string
  steps: string
  approvals: string
  /** Total seconds the session covers. */
  totalSeconds: number
  frames: ReplayFrame[]
}

const CONCUR_REPLAY: ReplayDetail = {
  id: 'run-concur-may',
  agentLabel: 'Cowork . File expenses',
  taskTitle: 'See my May invoices and file expenses on SAP Concur',
  harness: 'Claude Code',
  status: 'done',
  site: 'app.concur.com',
  startedAt: 'Jun 1, 2026',
  duration: '0:57',
  tokens: '4.3k',
  steps: '9',
  approvals: '1',
  totalSeconds: 60,
  frames: [
    {
      t: 0,
      kind: 'action',
      verb: 'navigate',
      node: 'concur.com',
      caption: 'Requesting permission to open concur.com',
    },
    {
      t: 2,
      kind: 'approval',
      verb: 'navigate',
      node: 'concur.com',
      caption: 'You allowed the agent to open concur.com',
      note: 'Allowed once',
    },
    {
      t: 4,
      kind: 'action',
      verb: 'navigate',
      node: 'concur.com',
      caption: 'Navigating to concur.com',
    },
    {
      t: 8,
      kind: 'action',
      verb: 'read',
      node: 'Concur home',
      caption: 'Session restored from vault, signed in as nikhil@example.com',
    },
    {
      t: 13,
      kind: 'action',
      verb: 'click',
      node: '"Create New Report"',
      caption: 'Opened the new-report form',
    },
    {
      t: 19,
      kind: 'action',
      verb: 'read',
      node: 'May invoices',
      caption: 'Matched 4 receipts from your invoices folder',
    },
    {
      t: 25,
      kind: 'action',
      verb: 'type',
      node: 'Report name',
      caption: 'Typed "May 2026 . Engineering"',
    },
    {
      t: 34,
      kind: 'action',
      verb: 'type',
      node: '4 expense lines',
      caption: 'Filled vendor, date, category and amount for 4 lines',
    },
    {
      t: 42,
      kind: 'action',
      verb: 'attach',
      node: '4 receipts',
      caption: 'Attached PDF receipts, total $1,284.50',
    },
    {
      t: 48,
      kind: 'approval',
      verb: 'submit',
      node: '"Submit Report"',
      caption: 'Approval requested: submit the report',
      note: 'Needs OK',
    },
    {
      t: 51,
      kind: 'approval',
      verb: 'submit',
      node: '"Submit Report"',
      caption: 'You allowed the submit once',
      note: 'Allowed once',
    },
    {
      t: 53,
      kind: 'action',
      verb: 'read',
      node: 'Confirmation',
      caption: 'Report submitted, #EXP-49217, routed to Dana R.',
    },
    {
      t: 56,
      kind: 'block',
      verb: 'click',
      node: '"Pay card balance"',
      caption: 'Blocked: payments are non-interactive for agents',
      note: 'Blocked',
    },
    {
      t: 58,
      kind: 'done',
      verb: 'done',
      node: '',
      caption: 'Run complete, expense report filed',
    },
  ],
}

/**
 * Per-run replay fixtures. Keys match the run ids surfaced from
 * `useRuns` so an Audit row click into `/governance/audit/:id/replay`
 * lands on real data for at least one run.
 */
const FIXTURES: Record<string, ReplayDetail> = {
  'run-concur-may': CONCUR_REPLAY,
}

const FALLBACK: ReplayDetail = {
  id: 'unknown',
  agentLabel: 'Unknown run',
  taskTitle: 'No replay was recorded for this run.',
  harness: 'Codex',
  status: 'stopped',
  site: 'about:blank',
  startedAt: '',
  duration: '0:00',
  tokens: '0',
  steps: '0',
  approvals: '0',
  totalSeconds: 0,
  frames: [],
}

interface UseReplayVariables {
  runId: string
}

export const useReplay = createQuery<ReplayDetail, UseReplayVariables>({
  queryKey: ['replay'],
  fetcher: ({ runId }) =>
    new Promise((resolve) =>
      setTimeout(() => resolve(FIXTURES[runId] ?? FALLBACK), 60),
    ),
})
