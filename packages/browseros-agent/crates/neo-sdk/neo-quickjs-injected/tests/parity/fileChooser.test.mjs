// page.on('filechooser') through the pinned Playwright and through Neo, on fresh
// tests/pages/file-chooser.html tabs (events.mjs): a click on a file input while the page listens
// reaches each wait once, as one FileChooser whose page() is the page, whose element() is that
// input (the same handle at every read), and whose isMultiple() says whether it takes several
// files (fileChooserFrames.test.mjs: inputs in frames). setFiles waits for Neo's setInputFiles
// (file-chooser.ts, Deviation), which a Neo-only case pins.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/page/page-filechooser.spec.ts "FC-021 should work for single file pick", "FC-016
    // should return the same file chooser when there are many watchdogs simultaneously",
    // "FC-002 should emit event once".
    name: 'a single-file input opens one chooser for every wait, its element that input',
    path: '/file-chooser.html',
    source: `const [first, second] = await Promise.all([
      page.waitForEvent('filechooser'),
      page.waitForEvent('filechooser'),
      page.click('#single'),
    ])
    return {
      same: first === second,
      multiple: first.isMultiple(),
      page: first.page() === page,
      id: await first.element().evaluate((input) => input.id),
      element: first.element() === first.element(),
      listenersLeft: page.listenerCount('filechooser'),
    }`,
    expected: {
      same: true,
      multiple: false,
      page: true,
      id: 'single',
      element: true,
      listenersLeft: 0,
    },
  },
  {
    // page-filechooser.spec.ts "FC-022 should work for \\"multiple\\"".
    name: 'a multiple input opens a chooser that takes several files',
    path: '/file-chooser.html',
    source: `const [chooser] = await Promise.all([page.waitForEvent('filechooser'), page.click('#multiple')])
    return { multiple: chooser.isMultiple(), id: await chooser.element().evaluate((input) => input.id) }`,
    expected: { multiple: true, id: 'multiple' },
  },
  {
    // Neo only (file-chooser.ts, Deviation): setFiles is setInputFiles, which Neo does not serve
    // yet, so it refuses rather than choose nothing.
    name: 'setFiles refuses until setInputFiles is served',
    path: '/file-chooser.html',
    source: `const [chooser] = await Promise.all([page.waitForEvent('filechooser'), page.click('#single')])
    return { setFiles: await settle(chooser.setFiles([])) }`,
    neo: (outcome) =>
      assert.deepEqual(outcome.value, { setFiles: { error: 'Error' } }),
  },
]

test('file choosers reach the same listeners the same way through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: { '/file-chooser.html': fixture('file-chooser.html') },
    cases,
  })
})
