/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { agentKeyFromSlug } from '../../src/domain/agent-key'
import { ownershipStore } from '../../src/domain/ownership'
import type { ToolCall } from '../../src/mcp/dispatch'
import { guardPageOwnership } from '../../src/mcp/guards/page-ownership'

const callerKey = agentKeyFromSlug('codex-bold-bison')
const ownerKey = agentKeyFromSlug('claude-swift-otter')

function call(): ToolCall {
  return {
    tool: { name: 'snapshot' } as never,
    args: { page: 12 },
    sessionId: 's1',
    identity: null,
    key: callerKey,
    agent: { agentId: callerKey, slug: 'codex' },
    agentLabel: 'Codex',
    session: {} as never,
    defaultTabGroupId: null,
    flags: { newPage: false, closePage: false, listTabs: false },
  }
}

afterEach(() => ownershipStore.clear())

describe('guardPageOwnership', () => {
  it('names the owning retained group in a foreign-page rejection', () => {
    ownershipStore.claimPage(ownerKey, 12)
    ownershipStore.setGroup(ownerKey, {
      id: 'G1',
      windowId: 1,
      color: 'red',
      title: 'claude/invoice-processing',
      collapsed: true,
    })

    const result = guardPageOwnership(call())
    expect(result).toMatchObject({ isError: true })
    expect(result?.content[0]).toMatchObject({
      text: 'page 12 is owned by claude/invoice-processing; call tabs new to open your own page',
    })
  })

  it('keeps unknown caller identities fail-open', () => {
    const unknown = call()
    unknown.agent = null
    unknown.key = null
    expect(guardPageOwnership(unknown)).toBeNull()
  })
})
