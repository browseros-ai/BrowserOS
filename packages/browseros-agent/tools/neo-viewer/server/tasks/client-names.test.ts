import { expect, test } from 'bun:test'
import { ClientNames } from './client-names'

const hints = (
  overrides: Partial<{
    connection: string | null
    pid: number | null
    userAgent: string | null
  }>,
) => ({
  connection: null,
  pid: null,
  userAgent: null,
  ...overrides,
})

test('what the call says wins, then its process, then its connection, then its User-Agent', () => {
  const names = new ClientNames(() => 0)
  names.remember(
    { name: 'by-connection' },
    hints({ connection: '127.0.0.1:50000' }),
  )
  names.remember({ name: 'by-pid' }, hints({ pid: 42 }))
  names.remember({ name: 'by-agent' }, hints({ userAgent: 'python/3' }))
  expect(names.resolve({ name: 'sent' }, hints({ pid: 42 })).name).toBe('sent')
  expect(
    names.resolve(null, hints({ pid: 42, connection: '127.0.0.1:50000' })).name,
  ).toBe('by-pid')
  expect(
    names.resolve(null, hints({ connection: '127.0.0.1:50000' })).name,
  ).toBe('by-connection')
  expect(names.resolve(null, hints({ userAgent: 'python/3' })).name).toBe(
    'by-agent',
  )
  expect(names.resolve(null, hints({ userAgent: 'curl/8.7.1' })).name).toBe(
    'curl',
  )
  expect(names.resolve(null, hints({})).name).toBe('unknown')
})

test('a name remembered by connection fades, since the port is reused once it closes', () => {
  let clock = 0
  const names = new ClientNames(() => clock)
  names.remember(
    { name: 'old owner' },
    hints({ connection: '127.0.0.1:50000' }),
  )
  clock = 10_000
  expect(
    names.resolve(null, hints({ connection: '127.0.0.1:50000' })).name,
  ).toBe('old owner')
  clock = 60_000
  expect(
    names.resolve(null, hints({ connection: '127.0.0.1:50000' })).name,
  ).toBe('unknown')
})
