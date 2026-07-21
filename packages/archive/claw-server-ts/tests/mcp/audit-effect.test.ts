import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'
import type { ToolCall } from '../../src/mcp/dispatch'
import { applyAudit } from '../../src/mcp/effects/audit'
import {
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../src/modules/db/db'
import { listDispatches } from '../../src/services/audit-log'

const tabs = BROWSER_TOOLS.find((tool) => tool.name === 'tabs')
if (!tabs) throw new Error('tabs tool missing')

function call(): ToolCall {
  return {
    tool: tabs,
    args: { action: 'list' },
    sessionId: 'session-a',
    identity: null,
    key: null,
    agent: { agentId: 'agent-a', slug: 'codex' },
    agentLabel: 'Codex',
    session: null,
    defaultTabGroupId: null,
    flags: { newPage: false, closePage: false, listTabs: true },
  }
}

describe('audit effect', () => {
  beforeEach(() => setAuditDbForTesting())
  afterEach(() => resetAuditDbForTesting())

  it('records ordinary tool errors and cancellations without a live tab', async () => {
    for (const cancelled of [false, true]) {
      await applyAudit({
        call: call(),
        result: {
          isError: true,
          content: [{ type: 'text', text: cancelled ? 'cancelled' : 'failed' }],
        },
        cancelled,
        durationMs: 5,
        startedAtMs: 1,
      })
    }

    const rows = listDispatches({ sessionId: 'session-a' }).rows
    expect(rows).toHaveLength(2)
    expect(
      rows.every((row) => JSON.parse(row.resultMeta ?? '{}').isError === true),
    ).toBe(true)
  })
})
