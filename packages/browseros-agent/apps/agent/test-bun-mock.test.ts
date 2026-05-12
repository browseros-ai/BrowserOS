import { afterEach, beforeEach, describe, expect, it, mock, vi } from 'bun:test'
import { createElement } from 'react'

vi.mock('@/lib/utils', () => ({
  cn: (...args: unknown[]) =>
    args.filter((a) => typeof a === 'string' && a.length > 0).join(' '),
}))

const mod = await import('react')

describe('test', () => {
  it('works', () => {
    expect(1 + 1).toBe(2)
  })
})
