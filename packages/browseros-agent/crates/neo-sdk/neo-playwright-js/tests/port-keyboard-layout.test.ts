import { expect, it } from 'bun:test'
import { staleOutputs } from '../scripts/port-keyboard-layout'

it('tests that the Rust keyboard tables match a fresh port of the vendored Playwright sources', () => {
  expect(staleOutputs()).toEqual([])
})
