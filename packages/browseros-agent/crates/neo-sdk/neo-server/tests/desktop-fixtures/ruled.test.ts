import { expect, test } from 'bun:test'
import { historicalFailures } from './cases/history'
import { strictFailures } from './cases/requirements'
import { baselineCases, cases } from './inventory'
import type { RuledCaseSpec } from './model'
import { ruledCases } from './ruled/cases'
import { validateRuledCases } from './ruled/requirements'

test('approved original tuples are immutable, pass-only and disjoint from the historical import', () => {
  expect(() => validateRuledCases(ruledCases, baselineCases)).not.toThrow()
  expect(cases).toHaveLength(256)
  expect(new Set(cases.map((spec) => spec.id)).size).toBe(256)
  expect(cases.filter((spec) => spec.expectation.kind === 'pass')).toHaveLength(
    256 - strictFailures.size,
  )
  expect(historicalFailures.size).toBe(6)
  expect(historicalFailures.has('capture-io-017')).toBe(true)
  expect(strictFailures.has('capture-io-017')).toBe(false)
  for (const field of ['name', 'file', 'id']) {
    const altered = ruledCases.map((spec) => structuredClone(spec))
    const first = altered[0]
    if (!first) throw new Error('Missing ruled case')
    if (field === 'name') first.name = 'Changed original title'
    else if (field === 'file') first.file = 'cases-click-recovery.ts'
    else first.id = 'act-001'
    expect(() => validateRuledCases(altered, baselineCases)).toThrow(
      'identities',
    )
  }
})

test('the bounded budget request and final uncertainty contract cannot join ordinary execution', () => {
  const terminal = cases.filter((spec) => spec.execution === 'terminal-select')
  expect(terminal.map((spec) => spec.id)).toEqual(['act-017'])
  expect(cases.filter((spec) => spec.execution === 'ordinary')).toHaveLength(
    253,
  )
  expect(
    cases.filter((spec) => spec.execution === 'density').map((spec) => spec.id),
  ).toEqual(['screenshot-scale-001', 'screenshot-scale-002'])
  const budget = ruledCases.find((spec) => spec.id === 'read-eval-022')
  expect(budget?.timeoutMs).toBe(600_000)
  expect(budget?.driverTimeoutMs).toBe(20_000)
  const wrongBudget = ruledCases.map((spec) =>
    spec.id === 'read-eval-022' ? { ...spec, timeoutMs: 20_000 } : spec,
  )
  expect(() => validateRuledCases(wrongBudget, baselineCases)).toThrow(
    'SDK600000',
  )
})

test('public rulings require pinned source and aliases retain fresh expanded identities', () => {
  for (const patch of [
    { commit: 'main' },
    { path: 'file:///tmp/private-report' },
    { claim: '' },
  ]) {
    const altered = ruledCases.map((spec, index): RuledCaseSpec => {
      if (index !== 0) return spec
      const first = spec.ruling.citations[0]
      return {
        ...spec,
        ruling: { ...spec.ruling, citations: [{ ...first, ...patch }] },
      }
    })
    expect(() => validateRuledCases(altered, baselineCases)).toThrow()
  }
  for (const aliases of [
    ['fill-008', 'fill-021', 'fill-034'],
    ['sdk-input-010', 'sdk-input-033'],
    ['sdk-read-eval-001', 'sdk-read-eval-002'],
    ['sdk-read-eval-003', 'sdk-read-eval-004'],
  ]) {
    const found = ruledCases.filter((spec) => aliases.includes(spec.id))
    expect(found).toHaveLength(aliases.length)
    expect(new Set(found.map((spec) => JSON.stringify(spec.steps))).size).toBe(
      1,
    )
  }
})
