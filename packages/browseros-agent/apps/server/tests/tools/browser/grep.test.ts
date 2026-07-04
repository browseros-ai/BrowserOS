import { describe, expect, it } from 'bun:test'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { executeTool } from '@browseros/browser-mcp/tools/framework'
import { grep } from '@browseros/browser-mcp/tools/grep'

function sessionWithSnapshot(text: string): BrowserSession {
  return {
    observe: () => ({ snapshot: async () => ({ text }) }),
    pages: { getInfo: () => ({ url: 'https://example.com' }) },
  } as unknown as BrowserSession
}

describe('grep tool', () => {
  it('returns matched lines with page, over, and count in structured output', async () => {
    const session = sessionWithSnapshot(
      'button "Save" [ref=e1]\nlink "Home"\nbutton "Save draft" [ref=e2]',
    )

    const result = await executeTool(
      grep,
      { page: 4, pattern: 'save', over: 'ax' },
      { session },
    )

    expect(result.isError).toBeFalsy()
    expect(result.structuredContent).toEqual({
      page: 4,
      over: 'ax',
      count: 2,
    })
  })

  it('reports zero matches without a matches array', async () => {
    const session = sessionWithSnapshot('link "Home"\nlink "About"')

    const result = await executeTool(
      grep,
      { page: 4, pattern: 'checkout', over: 'ax' },
      { session },
    )

    expect(result.isError).toBeFalsy()
    expect(result.structuredContent).toEqual({
      page: 4,
      over: 'ax',
      count: 0,
    })
  })
})
