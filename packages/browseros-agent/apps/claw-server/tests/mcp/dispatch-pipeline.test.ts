/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'
import { runEffects, runGuards, type ToolCall } from '../../src/mcp/dispatch'

const navigateTool = BROWSER_TOOLS.find((tool) => tool.name === 'navigate')
if (!navigateTool) throw new Error('navigate tool missing from registry')

function call(args: Record<string, unknown>): ToolCall {
  return {
    tool: navigateTool,
    args,
    sessionId: '',
    identity: null,
    key: null,
    agent: null,
    agentLabel: null,
    session: null,
    defaultTabGroupId: null,
    flags: { newPage: false, closePage: false, listTabs: false },
  }
}

describe('dispatch pipeline', () => {
  it('runs the navigate scheme guard before the browser connection guard', () => {
    const result = runGuards(call({ url: 'javascript:alert(1)' }))

    expect(result?.isError).toBe(true)
    expect(result?.content[0]).toEqual({
      type: 'text',
      text: 'navigate refuses javascript: URLs; only http(s) is allowed',
    })
  })

  it('rejects a blocked navigate scheme after leading whitespace', () => {
    const result = runGuards(call({ url: '  javascript:alert(1)' }))

    expect(result?.isError).toBe(true)
    expect(result?.content[0]).toEqual({
      type: 'text',
      text: 'navigate refuses javascript: URLs; only http(s) is allowed',
    })
  })

  it('isolates a throwing effect and returns the tool result', () => {
    const result = {
      content: [{ type: 'text' as const, text: 'ok' }],
      structuredContent: { page: 7 },
    }
    const warnings: Array<{
      message: string
      fields?: Record<string, unknown>
    }> = []

    const returned = runEffects(
      { call: call({}), result, cancelled: false, durationMs: 1 },
      [
        {
          name: 'audit',
          run: () => {
            throw new Error('disk unavailable')
          },
        },
      ],
      (message, fields) => warnings.push({ message, fields }),
    )

    expect(returned).toBe(result)
    expect(warnings).toEqual([
      {
        message: 'cockpit tool dispatch effect failed',
        fields: {
          tool: 'navigate',
          sessionId: undefined,
          effect: 'audit',
          error: 'disk unavailable',
        },
      },
    ])
  })
})
