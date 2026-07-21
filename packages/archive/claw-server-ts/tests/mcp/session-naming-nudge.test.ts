import { beforeEach, describe, expect, it } from 'bun:test'
import {
  buildSessionGroupTitle,
  type ClientIdentity,
  clientPrefixFromSlug,
  identityService,
} from '../../src/lib/mcp-session'
import type { ToolCall } from '../../src/mcp/dispatch'
import { applySessionNaming } from '../../src/mcp/effects/session-naming'
import type { ToolResult } from '../../src/mcp/register-fn'

function register(sessionId: string): ClientIdentity {
  return identityService.registerInitialize({
    sessionId,
    clientInfo: { name: 'Claude Code', version: '1.0.0' },
  })
}

function tipFor(identity: ClientIdentity): string {
  const title = buildSessionGroupTitle(
    clientPrefixFromSlug(identity.slug),
    identity.label,
  )
  return `Tip: this session is "${title}" — rename it with name_session name="<2-3 word task label>"`
}

function call(identity: ClientIdentity, toolName = 'snapshot'): ToolCall {
  return {
    tool: { name: toolName } as never,
    args: {},
    sessionId: identity.sessionId,
    identity,
    key: identity.key,
    agent: { agentId: identity.key, slug: identity.slug },
    agentLabel: identity.clientName,
    session: {} as never,
    defaultTabGroupId: null,
    flags: { newPage: false, closePage: false, listTabs: false },
  }
}

const ok: ToolResult = {
  content: [{ type: 'text', text: 'tool result' }],
  isError: false,
}

function apply(
  toolCall: ToolCall,
  result: ToolResult = ok,
): ToolResult | undefined {
  return applySessionNaming({
    call: toolCall,
    result,
    cancelled: false,
    durationMs: 1,
    startedAtMs: 1,
  })
}

describe('session naming nudge', () => {
  beforeEach(() => identityService.clear())

  it('appends the tip as a trailing text item on successive results', () => {
    const value = register('s1')
    const tip = tipFor(value)

    expect(apply(call(value, 'snapshot'))?.content).toEqual([
      { type: 'text', text: 'tool result' },
      { type: 'text', text: tip },
    ])
    expect(apply(call(value, 'read'))?.content).toEqual([
      { type: 'text', text: 'tool result' },
      { type: 'text', text: tip },
    ])
  })

  it('appends the tip last without splicing into existing text items', () => {
    const value = register('s1')
    const multi: ToolResult = {
      content: [
        { type: 'text', text: 'first' },
        { type: 'text', text: 'second' },
      ],
      isError: false,
    }

    expect(apply(call(value, 'read'), multi)?.content).toEqual([
      { type: 'text', text: 'first' },
      { type: 'text', text: 'second' },
      { type: 'text', text: tipFor(value) },
    ])
  })

  it('appends exactly five nudges then stays silent', () => {
    const value = register('s1')
    const toolCall = call(value, 'tabs')

    for (let index = 0; index < 5; index += 1) {
      expect(apply(toolCall)?.content).toEqual([
        { type: 'text', text: 'tool result' },
        { type: 'text', text: tipFor(value) },
      ])
    }
    expect(apply(toolCall)).toBeUndefined()
  })

  it('stops immediately after the session is renamed', () => {
    const value = register('s1')

    expect(apply(call(value))).toBeDefined()
    identityService.setLabel('s1', 'invoice-processing')
    expect(apply(call(value))).toBeUndefined()
  })

  it('skips errors and name_session without consuming nudges', () => {
    const value = register('s1')
    const toolCall = call(value)

    expect(apply(toolCall, { ...ok, isError: true })).toBeUndefined()
    expect(apply(call(value, 'name_session'))).toBeUndefined()

    for (let index = 0; index < 5; index += 1) {
      expect(apply(toolCall)).toBeDefined()
    }
    expect(apply(toolCall)).toBeUndefined()
  })

  it('appends the tip after image content when no text item exists', () => {
    const value = register('s1')
    const image = { type: 'image' as const, data: 'AAA', mimeType: 'image/png' }

    expect(
      apply(call(value, 'screenshot'), {
        content: [image],
        isError: false,
      })?.content,
    ).toEqual([image, { type: 'text', text: tipFor(value) }])
  })

  it('keeps independent counters for separate sessions', () => {
    const first = register('s1')
    const second = register('s2')

    for (let index = 0; index < 5; index += 1) {
      expect(apply(call(first))).toBeDefined()
      expect(apply(call(second))).toBeDefined()
    }
    expect(apply(call(first))).toBeUndefined()
    expect(apply(call(second))).toBeUndefined()
  })
})
