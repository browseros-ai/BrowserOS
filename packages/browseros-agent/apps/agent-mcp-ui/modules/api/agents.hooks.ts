import { nanoid } from 'nanoid'
import { createMutation, createQuery } from 'react-query-kit'
import type { RunStatus } from '@/lib/status'
import {
  buildCliCommand,
  buildMcpUrl,
  toSlug,
} from '@/screens/new-agent/new-agent.helpers'
import type { NewAgentValues } from '@/screens/new-agent/new-agent.schemas'

export interface AgentRow {
  id: string
  /** Display label, e.g. "Cowork . File expenses". */
  label: string
  harness: 'Claude Cowork' | 'Codex' | 'Hermes' | 'OpenClaw' | 'Gemini CLI'
  site: string
  task: string
  status: RunStatus
  liveLine: string
  /** Hex color used for the per-agent dot in cross-agent activity rows. */
  color: string
}

/**
 * Mock data. Shape matches what the eventual
 * `agent-mcp-interface` /agents endpoint will return. Replacing
 * `fetcher` with a real `$get`-then-parseResponse call is the only
 * change this hook needs when the backend route lands.
 */
const MOCK_AGENTS: AgentRow[] = [
  {
    id: 'cld-concur',
    label: 'Cowork . File expenses',
    harness: 'Claude Cowork',
    site: 'concur.com',
    task: 'See my May invoices and file expenses on SAP Concur',
    status: 'needs-ok',
    liveLine: 'Filling 4 expense lines',
    color: '#F26B2A',
  },
  {
    id: 'cld-li',
    label: 'Cowork . LinkedIn posts',
    harness: 'Claude Cowork',
    site: 'linkedin.com',
    task: 'Draft and queue 3 LinkedIn posts about the launch',
    status: 'running',
    liveLine: 'Typing the 2nd post in the composer',
    color: '#2F6FE0',
  },
  {
    id: 'cdx-sheet',
    label: 'Codex . Pricing research',
    harness: 'Codex',
    site: 'docs.google.com',
    task: 'Compile competitor pricing into a Google Sheet',
    status: 'running',
    liveLine: 'Pasting row 9 of 12 into the sheet',
    color: '#1F8A4C',
  },
]

export const useAgents = createQuery<AgentRow[]>({
  queryKey: ['agents'],
  fetcher: () =>
    new Promise((resolve) => setTimeout(() => resolve(MOCK_AGENTS), 60)),
})

export interface CreatedAgent {
  id: string
  name: string
  harness: NewAgentValues['harness']
  slug: string
  mcpUrl: string
  cliCommand: string
}

/**
 * Mock createAgent mutation. Mirrors the eventual hono-rpc surface so
 * swapping `mutationFn` for a real `$post`-then-parseResponse call is
 * a body-only change. The simulated latency keeps the optimistic UI
 * states honest.
 */
export const useCreateAgent = createMutation<CreatedAgent, NewAgentValues>({
  mutationFn: async (values) => {
    await new Promise((resolve) => setTimeout(resolve, 600))
    const slug = toSlug(values.name || values.harness)
    return {
      id: nanoid(8),
      name: values.name,
      harness: values.harness,
      slug,
      mcpUrl: buildMcpUrl(slug),
      cliCommand: buildCliCommand(slug),
    }
  },
})
