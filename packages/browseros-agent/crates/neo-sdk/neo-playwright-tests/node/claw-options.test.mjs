import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import test from 'node:test'
import { clawContextOptions, clawDefaults } from './claw-context-options.mjs'
import { defaultOwnWindowPages, runnerOptions } from './options.mjs'

test('tests that each engine is selected without forwarding the switch', () => {
  assert.equal(runnerOptions([]).engine, 'claw')
  for (const engine of ['reference', 'claw', 'neo']) {
    for (const args of [['--engine', engine], [`--engine=${engine}`]]) {
      const options = runnerOptions(args)
      assert.equal(options.engine, engine)
      assert.equal(options.reference, engine === 'reference')
      assert.equal(options.expectLoop, engine !== 'reference')
      assert.ok(!options.args.some((arg) => arg.includes('--engine')))
    }
  }
  assert.equal(runnerOptions(['--reference']).engine, 'reference')
  assert.throws(() => runnerOptions(['--engine', 'other']), /engine/)
  assert.throws(() => runnerOptions(['--engine']), /engine/)
  assert.throws(
    () => runnerOptions(['--reference', '--engine', 'claw']),
    /Conflicting/,
  )
})

test('tests that v2 fails before importing absent SDK files or preparing a run', () => {
  const child = spawnSync(
    process.execPath,
    [new URL('./run.mjs', import.meta.url).pathname, '--engine', 'neo'],
    { encoding: 'utf8' },
  )
  assert.equal(child.status, 1)
  assert.match(child.stderr, /Neo v2's SDK is not on this branch/)
  assert.doesNotMatch(child.stderr, /ENOENT|ERR_MODULE_NOT_FOUND/)
})

test('tests that only marked, unchanged implicit defaults disappear', () => {
  const implicit = new Set(Object.keys(clawDefaults))
  assert.deepEqual(
    clawContextOptions({ ...clawDefaults }, undefined, implicit),
    {},
  )
  for (const [name, value] of Object.entries(clawDefaults)) {
    assert.deepEqual(
      clawContextOptions({ ...clawDefaults }, { [name]: value }, implicit),
      { [name]: value },
    )
    assert.deepEqual(
      clawContextOptions(
        { ...clawDefaults },
        undefined,
        new Set([...implicit].filter((key) => key !== name)),
      ),
      { [name]: value },
    )
  }
  assert.deepEqual(
    clawContextOptions(
      {
        ...clawDefaults,
        viewport: { width: 400, height: 300 },
        recordHar: { path: 'test.har' },
      },
      undefined,
      implicit,
    ),
    { viewport: { width: 400, height: 300 }, recordHar: { path: 'test.har' } },
  )
  assert.deepEqual(
    clawContextOptions(clawDefaults, { viewport: null }, implicit),
    { viewport: null },
  )
})

test('tests that hosted mode is an explicit launch option for every engine', () => {
  assert.equal(runnerOptions([]).hostedMode, false)
  for (const engine of ['reference', 'claw', 'neo']) {
    const options = runnerOptions(['--hosted-mode', '--engine', engine])
    assert.equal(options.hostedMode, true)
    assert.ok(!options.args.includes('--hosted-mode'))
  }
})

test('tests that only Linux opens pages in a window of their own unless asked', () => {
  assert.equal(defaultOwnWindowPages('linux'), true)
  assert.equal(defaultOwnWindowPages('darwin'), false)
  assert.equal(runnerOptions([]).ownWindowPages, defaultOwnWindowPages())
  for (const [flag, value] of [
    ['--own-window-pages', true],
    ['--background-tabs', false],
  ]) {
    const options = runnerOptions([flag])
    assert.equal(options.ownWindowPages, value)
    assert.ok(!options.args.includes(flag))
  }
})
