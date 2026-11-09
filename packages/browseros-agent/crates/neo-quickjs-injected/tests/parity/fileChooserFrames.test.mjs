// page.on('filechooser') for inputs in frames, through the pinned Playwright and through Neo, on
// fresh tests/pages/file-chooser-frames.html tabs (events.mjs): a click on a file input in a
// same-origin frame, or in a cross-site frame Chromium runs out of process, opens a chooser on
// the page's listeners, whose element() is that input, in its frame. Playwright intercepts file
// choosers on each frame session, and Chromium reports an out-of-process frame's chooser only to
// a session intercepting that frame's own; without it the browser's own dialog would open.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/page/page-filechooser.spec.ts "FC-004 should emit event for iframe".
    name: 'an input in a same-origin frame opens a chooser on the page, its element in that frame',
    path: '/file-chooser-frames.html',
    source: `const [chooser] = await Promise.all([
      page.waitForEvent('filechooser'),
      page.frameLocator('#same').locator('#inner').click(),
    ])
    return {
      page: chooser.page() === page,
      id: await chooser.element().evaluate((input) => input.id),
      inFrame: await chooser.element().evaluate((input) => input.ownerDocument !== window.top.document),
    }`,
    expected: { page: true, id: 'inner', inFrame: true },
  },
  {
    // tests/library/chromium/oopif.spec.ts "OO-023 should emit filechooser event for iframe".
    name: 'an input in a cross-site frame opens a chooser on the page, its element in that frame',
    path: '/file-chooser-frames.html',
    // Neo: arm interception before the replacement OOPIF attaches, so the click cannot
    // overtake the frame session's subscription setup.
    source: `const waiting = page.waitForEvent('filechooser')
    await page.reload()
    const [chooser] = await Promise.all([
      waiting,
      page.frameLocator('#cross').locator('#cross-input').click(),
    ])
    return {
      page: chooser.page() === page,
      multiple: chooser.isMultiple(),
      id: await chooser.element().evaluate((input) => input.id),
      host: await chooser.element().evaluate(() => location.hostname),
    }`,
    expected: {
      page: true,
      multiple: true,
      id: 'cross-input',
      host: 'localhost',
    },
  },
]

test('file choosers in frames reach the page the same way through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/file-chooser-frames.html': fixture('file-chooser-frames.html'),
      '/chooser-input.html':
        '<!doctype html><title>chooser input</title><input type="file" id="cross-input" multiple>',
    },
    cases,
  })
})
