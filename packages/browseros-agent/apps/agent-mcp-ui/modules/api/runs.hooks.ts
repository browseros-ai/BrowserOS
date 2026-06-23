import { createQuery } from 'react-query-kit'
import type { RunStatus } from '@/lib/status'
import type { Harness } from '@/screens/new-agent/new-agent.schemas'

export type RunHarness = Harness

export interface RunRow {
  id: string
  agentId: string
  /** Human label, e.g. "Cowork . File expenses". */
  agentLabel: string
  harness: RunHarness
  status: RunStatus
  /** One-line task description. */
  title: string
  /** Relative time, e.g. "2m ago", "1h ago", "Yesterday 17:42". */
  when: string
  /** Number of recorded actions. */
  actions: number
  /** Wall-clock duration string, e.g. "4m 12s". Empty if still running. */
  duration: string
  /** Primary site touched during the run. */
  site: string
  /** Short summary used by filters and replay captions. */
  summary: string
}

/**
 * Mock audit data. Mirrors the eventual shape from
 * `agent-mcp-interface` so the only change is `fetcher` swapping to a
 * real `$get`-then-parseResponse call when the route lands.
 */
const MOCK_RUNS: RunRow[] = [
  {
    id: 'run-concur-may',
    agentId: 'cld-concur',
    agentLabel: 'Cowork . File expenses',
    harness: 'Claude Code',
    status: 'needs-ok',
    title: 'See my May invoices and file expenses on SAP Concur',
    when: '2m ago',
    actions: 18,
    duration: '3m 41s',
    site: 'concur.com',
    summary: 'approval requested · 4 expense lines',
  },
  {
    id: 'run-linkedin',
    agentId: 'cld-li',
    agentLabel: 'Cowork . LinkedIn posts',
    harness: 'Claude Code',
    status: 'running',
    title: 'Draft and queue 3 LinkedIn posts about the launch',
    when: '4m ago',
    actions: 12,
    duration: '',
    site: 'linkedin.com',
    summary: 'typing in composer',
  },
  {
    id: 'run-pricing-sheet',
    agentId: 'cdx-sheet',
    agentLabel: 'Codex . Pricing research',
    harness: 'Codex',
    status: 'running',
    title: 'Compile competitor pricing into a Google Sheet',
    when: '8m ago',
    actions: 27,
    duration: '',
    site: 'docs.google.com',
    summary: 'pasting row 9 of 12',
  },
  {
    id: 'run-hubspot-leads',
    agentId: 'cdx-leads',
    agentLabel: 'Codex . Pipeline digest',
    harness: 'Codex',
    status: 'done',
    title: "Pull this week's leads from HubSpot into a summary",
    when: '34m ago',
    actions: 41,
    duration: '6m 12s',
    site: 'app.hubspot.com',
    summary: 'summary posted',
  },
  {
    id: 'run-stripe-block',
    agentId: 'hrm-stripe',
    agentLabel: 'Hermes . Refunds',
    harness: 'Hermes',
    status: 'blocked',
    title: 'Refund the duplicate charge from yesterday',
    when: '1h ago',
    actions: 5,
    duration: '0m 48s',
    site: 'stripe.com',
    summary: 'ACL block: payments & checkout',
  },
  {
    id: 'run-amazon-restock',
    agentId: 'cld-amazon',
    agentLabel: 'Cowork . Office restock',
    harness: 'Claude Code',
    status: 'needs-human',
    title: 'Restock the team snacks and ship to the office',
    when: '2h ago',
    actions: 22,
    duration: '4m 02s',
    site: 'amazon.com',
    summary: 'captcha · handoff to user',
  },
  {
    id: 'run-salesforce-clean',
    agentId: 'cdx-sfdc',
    agentLabel: 'Codex . CRM hygiene',
    harness: 'Codex',
    status: 'done',
    title: 'Dedupe leads and fix missing region tags',
    when: 'Yesterday 17:42',
    actions: 64,
    duration: '11m 06s',
    site: 'salesforce.com',
    summary: '52 records updated',
  },
  {
    id: 'run-notion-recap',
    agentId: 'hrm-notion',
    agentLabel: 'Hermes . Weekly recap',
    harness: 'Hermes',
    status: 'stopped',
    title: 'Draft the weekly recap doc with last week metrics',
    when: 'Yesterday 09:11',
    actions: 8,
    duration: '1m 22s',
    site: 'notion.so',
    summary: 'user stopped at action 8',
  },
]

export const useRuns = createQuery<RunRow[]>({
  queryKey: ['runs'],
  fetcher: () =>
    new Promise((resolve) => setTimeout(() => resolve(MOCK_RUNS), 60)),
})
