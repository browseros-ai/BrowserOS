import { expect, it } from 'bun:test'
import { MAX_REPLY_DEPTH } from '../src/probe/generated'
import { replyDepth, replyText } from '../src/probe/reply-text'

it('repairs lone halves in values and keys while preserving pairs and literal escapes', () => {
  const high = String.fromCharCode(0xd83d)
  const low = String.fromCharCode(0xde00)
  const pair = high + low
  const literal = '\\ud83d'
  const value = {
    high,
    low,
    pair,
    literal,
    prefixed: '\\' + high,
    [high]: low,
  }
  const text = replyText(value)
  expect(JSON.parse(text)).toEqual({
    high: '\ufffd',
    low: '\ufffd',
    pair,
    literal,
    prefixed: '\\\ufffd',
    ['\ufffd']: '\ufffd',
  })
  expect(text).toBe(JSON.stringify(JSON.parse(text)))
})

it('counts brackets outside strings at the shared 297-level bound', () => {
  for (const depth of [297, 298]) {
    const text = '['.repeat(depth) + '"[\\"]"' + ']'.repeat(depth)
    expect(replyDepth(text)).toBe(depth)
  }
  expect(MAX_REPLY_DEPTH).toBe(297)
})

const nested = (depth: number): unknown => {
  let value: unknown = 0
  for (let index = 0; index < depth; index++) value = [value]
  return value
}

it('keeps an allowed reply and answers an overdeep snapshot with the two ways out', () => {
  const allowed = { kind: 'snapshot', json: nested(MAX_REPLY_DEPTH - 1) }
  expect(replyText(allowed)).toBe(JSON.stringify(allowed))
  const refused = JSON.parse(
    replyText({ kind: 'snapshot', json: nested(MAX_REPLY_DEPTH) }),
  )
  expect(refused).toMatchObject({ kind: 'error', class: 'unsupported' })
  expect(refused.treeDepth).toBe(148)
  expect(refused.message).toContain('nested about 148 levels deep')
  expect(refused.message).toContain('about 147')
  expect(refused.message).toContain('140 or less')
  expect(refused.message).toContain('{ depth: n }')
  expect(refused.message).toContain("locator's snapshot")
})

it('omits an overdeep optional cover tree and keeps its other facts', () => {
  const reply = {
    kind: 'cover',
    facts: { name: 'Read first', snapshot: nested(298) },
  }
  expect(JSON.parse(replyText(reply))).toEqual({
    kind: 'cover',
    facts: { name: 'Read first', snapshot: null },
  })
  expect(reply.facts.snapshot).not.toBeNull()
})

it('returns fixed error text if the serializer cannot produce a reply', () => {
  const cyclic: { self?: unknown } = {}
  cyclic.self = cyclic
  expect(JSON.parse(replyText(cyclic))).toEqual({
    kind: 'error',
    class: 'internal',
    message: 'Neo could not serialize the probe reply.',
  })
})
