import { expect, test } from 'bun:test'
import { bodyProbe } from './probes'

test('tests that a page verdict remains small with rejection beyond the maximum inline size', () => {
  const code = bodyProbe({
    submitSelector: '#submit',
    acceptedPattern: 'ACCEPTED',
    rejectedPattern: 'REJECTED',
  })
  const probe = new Function('document', code)
  expect(probe({ body: { innerText: `${'x'.repeat(250_000)}REJECTED` } })).toBe(
    'rejected',
  )
  expect(probe({ body: { innerText: 'ACCEPTED' } })).toBe('accepted')
  expect(probe({ body: { innerText: 'waiting' } })).toBeNull()
})

test('tests that verdict patterns with quotes and backslashes survive transport', () => {
  const code = bodyProbe({
    submitSelector: '#submit',
    acceptedPattern: '"success"\\s*:\\s*true',
  })
  expect(
    new Function('document', code)({
      body: { innerText: '{"success": true}' },
    }),
  ).toBe('accepted')
})
