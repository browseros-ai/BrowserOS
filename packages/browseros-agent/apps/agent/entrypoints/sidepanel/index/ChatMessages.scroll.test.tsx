/**
 * Task 3 structural test: verifies ChatMessages.tsx imports and uses
 * AssistantMessageBody for assistant content while keeping actions outside.
 *
 * This is a source-level contract test because bun test cannot resolve
 * the @/ path aliases used by ChatMessages.tsx's transitive dependencies.
 */
import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const chatMessagesPath = resolve(
  import.meta.dir,
  'ChatMessages.tsx',
)
const source = readFileSync(chatMessagesPath, 'utf-8')

describe('ChatMessages assistant scroll-body wiring (Task 3)', () => {
  it('imports AssistantMessageBody from the message primitives', () => {
    expect(source).toMatch(
      /import\s*\{[^}]*AssistantMessageBody[^}]*\}\s*from\s*['"]@\/components\/ai-elements\/message['"]/,
    )
  })

  it('wraps assistant content inside AssistantMessageBody', () => {
    // Should contain a conditional that wraps segments in AssistantMessageBody
    // for assistant role messages
    expect(source).toMatch(
      /message\.role\s*===\s*['"]assistant['"]/,
    )
    expect(source).toMatch(/AssistantMessageBody/)
  })

  it('keeps ChatMessageActions outside AssistantMessageBody', () => {
    // Find the last closing AssistantMessageBody tag and the ChatMessageActions JSX usage
    const lastBodyClose = source.lastIndexOf('</AssistantMessageBody>')
    expect(lastBodyClose).toBeGreaterThan(-1)

    // ChatMessageActions as JSX usage (not import) — find '<ChatMessageActions'
    const actionsJsxIdx = source.indexOf('<ChatMessageActions')
    expect(actionsJsxIdx).toBeGreaterThan(-1)
    expect(actionsJsxIdx).toBeGreaterThan(lastBodyClose)
  })
})
