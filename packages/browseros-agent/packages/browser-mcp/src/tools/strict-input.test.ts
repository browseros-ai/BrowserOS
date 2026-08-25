import { describe, expect, it } from 'bun:test'
import { BROWSER_TOOLS } from './registry'

describe('browser tool input schemas', () => {
  it('rejects unknown arguments on every tool', () => {
    const permissive: string[] = []
    for (const tool of BROWSER_TOOLS) {
      const parsed = tool.input.safeParse({
        ...sampleFor(tool.name),
        __unknown_argument__: true,
      })
      if (parsed.success) {
        permissive.push(tool.name)
      }
    }
    expect(permissive).toEqual([])
  })

  it('rejects unknown arguments nested inside object and array fields', () => {
    const screenshot = BROWSER_TOOLS.find((tool) => tool.name === 'screenshot')
    // A misspelled nested dimension must not be silently dropped and defaulted.
    expect(
      screenshot?.input.safeParse({
        page: 1,
        size: { width: 100, heigth: 200 },
      }).success,
    ).toBe(false)

    const act = BROWSER_TOOLS.find((tool) => tool.name === 'act')
    expect(
      act?.input.safeParse({
        page: 1,
        kind: 'fill',
        fields: [{ ref: 'e1', value: 'x', typo: true }],
      }).success,
    ).toBe(false)
  })
})

/** Minimal valid arguments per tool, so the only parse failure can be the unknown key. */
function sampleFor(name: string): Record<string, unknown> {
  switch (name) {
    case 'act':
      return { page: 1, kind: 'click', ref: 'e1' }
    case 'grep':
      return { page: 1, pattern: 'x' }
    case 'navigate':
      return { page: 1, action: 'reload' }
    case 'evaluate':
      return { page: 1, code: 'return 1' }
    case 'run':
      return { code: 'return 1' }
    case 'wait':
      return { page: 1, for: 'text', value: 'x' }
    case 'upload':
      return { page: 1, ref: 'e1', file: '/tmp/x' }
    case 'download':
      return { page: 1, ref: 'e1' }
    case 'tabs':
      return { action: 'list' }
    case 'windows':
      return { action: 'list' }
    case 'tab_groups':
      return { action: 'list' }
    case 'history':
      return {}
    default:
      return { page: 1 }
  }
}
