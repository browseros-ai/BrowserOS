/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { ConversationPresence } from '../../../src/api/services/conversation-presence'

describe('ConversationPresence', () => {
  it('buffers tab effects while a run is prepared but not yet visible', async () => {
    const presence = new ConversationPresence()
    const reader = presence.subscribe().getReader()
    await reader.read()

    presence.prepareRun({
      conversationId: 'preparing',
      runId: 'preparing-run',
      tabIds: [8],
    })
    expect(
      presence.touchTab({
        conversationId: 'preparing',
        runId: 'preparing-run',
        tabId: 9,
      }),
    ).toBe(true)
    expect(presence.snapshot()).toEqual({ runs: [], tabs: [] })

    presence.startRun({
      conversationId: 'preparing',
      runId: 'preparing-run',
      tabIds: [8],
    })

    expect((await reader.read()).value).toMatchObject({
      type: 'run-started',
      run: { tabIds: [8, 9] },
    })
    await reader.cancel()
  })

  it('discards an unpublished prepared run when setup fails', () => {
    const presence = new ConversationPresence()
    presence.prepareRun({
      conversationId: 'failed-setup',
      runId: 'failed-run',
      tabIds: [7],
    })

    expect(presence.finishRun('failed-setup', 'failed-run', 'failed')).toBe(
      true,
    )
    expect(presence.snapshot()).toEqual({ runs: [], tabs: [] })
  })

  it('retains the latest conversation view for every touched tab', () => {
    const presence = new ConversationPresence()
    presence.startRun({
      conversationId: 'conversation-1',
      runId: 'run-1',
      tabIds: [10, 11, 10],
    })
    presence.touchTab({
      conversationId: 'conversation-1',
      runId: 'run-1',
      tabId: 12,
    })
    presence.finishRun('conversation-1', 'run-1', 'completed')

    expect(presence.snapshot().tabs).toEqual([
      expect.objectContaining({
        tabId: 10,
        conversationId: 'conversation-1',
        status: 'completed',
      }),
      expect.objectContaining({
        tabId: 11,
        conversationId: 'conversation-1',
        status: 'completed',
      }),
      expect.objectContaining({
        tabId: 12,
        conversationId: 'conversation-1',
        status: 'completed',
      }),
    ])
  })

  it('multicasts a later run to every tab still associated with the conversation', () => {
    const presence = new ConversationPresence()
    presence.startRun({
      conversationId: 'conversation-1',
      runId: 'first-run',
      tabIds: [10],
    })
    presence.touchTab({
      conversationId: 'conversation-1',
      runId: 'first-run',
      tabId: 11,
    })
    presence.finishRun('conversation-1', 'first-run', 'completed')

    presence.startRun({
      conversationId: 'conversation-1',
      runId: 'second-run',
      tabIds: [10],
    })

    expect(presence.snapshot().runs[0]?.tabIds).toEqual([10, 11])
    expect(presence.snapshot().tabs).toEqual([
      expect.objectContaining({ tabId: 10, runId: 'second-run' }),
      expect.objectContaining({ tabId: 11, runId: 'second-run' }),
    ])
  })

  it('does not reclaim an associated tab that a newer conversation took over', () => {
    const presence = new ConversationPresence()
    presence.startRun({
      conversationId: 'older',
      runId: 'older-first-run',
      tabIds: [20, 21],
    })
    presence.finishRun('older', 'older-first-run', 'completed')
    presence.startRun({
      conversationId: 'newer',
      runId: 'newer-run',
      tabIds: [21],
    })

    presence.startRun({
      conversationId: 'older',
      runId: 'older-second-run',
      tabIds: [20],
    })

    expect(presence.snapshot().tabs).toEqual([
      expect.objectContaining({
        tabId: 20,
        conversationId: 'older',
        runId: 'older-second-run',
      }),
      expect.objectContaining({
        tabId: 21,
        conversationId: 'newer',
        runId: 'newer-run',
      }),
    ])
  })

  it('ignores stale or uncorrelated browser effects', () => {
    const presence = new ConversationPresence()
    presence.startRun({
      conversationId: 'conversation-2',
      runId: 'current-run',
      tabIds: [],
    })

    expect(
      presence.touchTab({
        conversationId: 'conversation-2',
        runId: 'old-run',
        tabId: 20,
      }),
    ).toBe(false)
    expect(
      presence.touchTab({
        conversationId: 'conversation-2',
        tabId: 21,
      }),
    ).toBe(false)
    expect(presence.snapshot().tabs).toEqual([])
  })

  it('sends an atomic snapshot before live events to new subscribers', async () => {
    const presence = new ConversationPresence()
    presence.startRun({
      conversationId: 'conversation-3',
      runId: 'run-3',
      tabIds: [30],
    })
    const reader = presence.subscribe().getReader()

    expect((await reader.read()).value).toMatchObject({
      type: 'snapshot',
      tabs: [{ tabId: 30, conversationId: 'conversation-3' }],
    })
    presence.touchTab({
      conversationId: 'conversation-3',
      runId: 'run-3',
      tabId: 31,
    })
    expect((await reader.read()).value).toMatchObject({
      type: 'tab-touched',
      tab: { tabId: 31, conversationId: 'conversation-3' },
    })
    await reader.cancel()
  })

  it('forgets only tab mappings still owned by the deleted conversation', () => {
    const presence = new ConversationPresence()
    presence.startRun({
      conversationId: 'older',
      runId: 'old-run',
      tabIds: [40, 41],
    })
    presence.startRun({
      conversationId: 'newer',
      runId: 'new-run',
      tabIds: [41],
    })

    presence.forgetConversation('older')

    expect(presence.snapshot().tabs).toEqual([
      expect.objectContaining({ tabId: 41, conversationId: 'newer' }),
    ])
  })
})
