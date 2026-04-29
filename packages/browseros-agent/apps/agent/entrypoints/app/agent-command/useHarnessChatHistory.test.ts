import { describe, expect, it } from 'bun:test'
import { mapHarnessHistoryPage } from './harness-history-mapper'

describe('mapHarnessHistoryPage', () => {
  it('maps rich harness history into chat history items', () => {
    const page = mapHarnessHistoryPage({
      agentId: 'agent-1',
      sessionId: 'main',
      items: [
        {
          id: 'agent:agent-1:main:1',
          agentId: 'agent-1',
          sessionId: 'main',
          role: 'assistant',
          text: 'Done.',
          createdAt: 1000,
          reasoning: { text: 'checking state' },
          toolCalls: [
            {
              toolCallId: 'tool-1',
              toolName: 'read_file',
              status: 'completed',
              input: { path: 'src/index.ts' },
              output: 'file contents',
            },
          ],
        },
      ],
    })

    expect(page.items).toEqual([
      {
        id: 'agent:agent-1:main:1',
        role: 'assistant',
        text: 'Done.',
        timestamp: 1000,
        messageSeq: 1,
        sessionKey: 'main',
        source: 'user-chat',
        reasoning: { text: 'checking state' },
        toolCalls: [
          {
            toolCallId: 'tool-1',
            toolName: 'read_file',
            label: 'Read file',
            subject: 'index.ts',
            status: 'completed',
            input: { path: 'src/index.ts' },
            output: 'file contents',
          },
        ],
      },
    ])
  })
})
