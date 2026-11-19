import { expect, test } from 'bun:test'
import { bodyProbe, submitProbe } from './probes'

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

test('tests that a page without a body yet is no verdict rather than an error', () => {
  const code = bodyProbe({
    submitSelector: '#submit',
    acceptedPattern: 'ACCEPTED',
    rejectedPattern: 'REJECTED',
  })
  expect(new Function('document', code)({ body: null })).toBeNull()
})

test('tests that submit skips a page that already accepted, or has no control to click', () => {
  const clicks: string[] = []
  const submit = (site: Parameters<typeof submitProbe>[0], text: string) =>
    new Function('document', submitProbe(site))({
      body: { innerText: text },
      querySelector: (selector: string) => ({
        click: () => clicks.push(selector),
      }),
    })
  const site = {
    submitSelector: '#go',
    acceptedPattern: 'Verification Success',
  }
  expect(submit(site, 'Verification Success... Hooray!')).toBe(false)
  expect(submit({ acceptedPattern: 'ok' }, 'waiting')).toBe(false)
  expect(clicks).toEqual([])
  expect(submit(site, 'waiting')).toBe(true)
  expect(clicks).toEqual(['#go'])
})
