/** Neo: Selection stays inside the corpus, and explicit defaults cannot be silently omitted. */
import assert from 'node:assert/strict'
import test from 'node:test'
import { contextOptions } from './context-options.mjs'
import { runnerOptions } from './options.mjs'

test('full corpus defaults and all worker spellings keep the browser cap', () => {
  assert.equal(runnerOptions([]).workers, 4)
  for (const args of [
    ['--workers', '2'],
    ['--workers=2'],
    ['-j', '2'],
    ['-j2'],
  ])
    assert.equal(runnerOptions(args).workers, 2)
  for (const args of [
    ['--workers', '5'],
    ['-j8'],
    ['--workers=50%'],
    ['--workers', '0'],
  ])
    assert.throws(() => runnerOptions(args), /between 1 and 4/)
})

test('file and grep selection cannot re-enable other headful cases', () => {
  for (const args of [
    [],
    ['--grep', 'HE-'],
    ['--grep-invert', 'example'],
    ['-Gexample'],
    ['--', 'headful.spec.ts'],
  ]) {
    const parsed = runnerOptions(args).args
    const reject = new RegExp(parsed[parsed.indexOf('--grep-invert') + 1])
    assert.equal(
      reject.test(
        'browseros-neo headful.spec.ts HE-012 Page.bringToFront should work',
      ),
      false,
    )
    assert.equal(
      reject.test('browseros-neo headful.spec.ts HE-011 other test'),
      true,
    )
    assert.equal(
      reject.test('browseros-neo page-click.spec.ts CL-001 button'),
      false,
    )
    if (parsed.includes('--'))
      assert.ok(parsed.indexOf('--grep-invert') < parsed.indexOf('--'))
  }
})

test('headless ownership and the JSON report cannot be overridden', () => {
  for (const arg of [
    '--headed',
    '--config=elsewhere',
    '-celsewhere',
    '--reporter=line',
    '--output=elsewhere',
    '-uall',
    '--debug',
  ])
    assert.throws(() => runnerOptions([arg]), /contract/)
  assert.equal(runnerOptions(['--debug-build']).debugBuild, true)
  assert.equal(runnerOptions(['--debug-build', '--release']).debugBuild, false)
})

test('only implicit offline and download defaults are omitted', () => {
  const combined = { offline: false, acceptDownloads: true, locale: 'en-US' }
  const implicit = new Set(['offline', 'acceptDownloads'])
  assert.deepEqual(contextOptions(combined, undefined, implicit), {
    locale: 'en-US',
  })
  assert.deepEqual(contextOptions(combined, { offline: false }, implicit), {
    offline: false,
    locale: 'en-US',
  })
  assert.deepEqual(
    contextOptions(combined, { acceptDownloads: true }, implicit),
    { acceptDownloads: true, locale: 'en-US' },
  )
  assert.deepEqual(contextOptions(combined, undefined, new Set()), combined)
  assert.deepEqual(
    contextOptions(
      { offline: true, acceptDownloads: false },
      undefined,
      implicit,
    ),
    { offline: true, acceptDownloads: false },
  )
  assert.deepEqual(combined, {
    offline: false,
    acceptDownloads: true,
    locale: 'en-US',
  })
})
