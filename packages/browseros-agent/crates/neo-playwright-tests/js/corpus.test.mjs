/** Neo: Corpus coverage is independent of current SDK capabilities. */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { corpus } from './prepare.mjs'

test('all page suites and three tab suites are included with real feature families', () => {
  assert.equal(corpus.suites.length, 139)
  assert.equal(new Set(corpus.suites.map((suite) => suite.file)).size, 139)
  assert.equal(
    corpus.suites.filter((suite) => suite.file.startsWith('tests/page/'))
      .length,
    136,
  )
  for (const suite of corpus.suites) {
    assert.equal(suite.include, true, suite.file)
    assert.ok(
      suite.feature && !['excluded', 'unclassified'].includes(suite.feature),
      suite.file,
    )
    assert.match(suite.sha256, /^[a-f0-9]{64}$/)
    assert.ok(suite.reason)
  }
  assert.deepEqual(
    corpus.suites
      .filter((suite) => suite.title)
      .map(({ file, title }) => ({ file, title })),
    [
      {
        file: 'tests/library/headful.spec.ts',
        title: 'HE-012 Page.bringToFront should work',
      },
    ],
  )
})
