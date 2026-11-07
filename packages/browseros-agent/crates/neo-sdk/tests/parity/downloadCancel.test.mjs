// download.cancel() through the pinned Playwright and through Neo, on fresh
// tests/pages/download.html tabs (events.mjs): a download whose body the server never finishes
// (/slow-download sends 64 KiB of 2 MB, then holds) is still in flight when cancel() is called;
// it then fails as 'canceled', the state Chromium reports and Playwright passes on, and path()
// rejects. A download already finished stays finished: cancel() resolves, failure() stays null
// and its file stays readable until delete(). A script reads a file back through its page
// (harness.mjs `fileContent`), never returning a path, which each engine keeps somewhere else.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { fileContent, skip } from './harness.mjs'

/** The size the slow download announces, and how much of it the server sends. */
const SLOW_SIZE = 2_000_000
const SLOW_SENT = 65_536

/** An attachment whose body stops after SLOW_SENT bytes and never ends. */
function slowDownload(_request, response) {
  response.writeHead(200, {
    'content-type': 'application/octet-stream',
    'content-disposition': 'attachment; filename="slow.bin"',
    'content-length': String(SLOW_SIZE),
  })
  response.write(Buffer.alloc(SLOW_SENT, 97))
}

const cases = [
  {
    // tests/library/download.spec.ts "DO-027 should be able to cancel pending downloads".
    name: "a download cancelled in flight fails as 'canceled', and path() rejects",
    path: '/download.html',
    source: `const [download] = await Promise.all([page.waitForEvent('download'), page.click('#slow')])
    const canceled = await settle(download.cancel())
    return {
      download: [path(download.url()), download.suggestedFilename(), download.page() === page],
      canceled,
      failure: await download.failure(),
      path: await settle(download.path()),
    }`,
    expected: {
      download: ['/slow-download', 'slow.bin', true],
      canceled: 'undefined',
      failure: 'canceled',
      path: { error: 'Error' },
    },
    probe: 'location.pathname',
    effect: '/download.html',
  },
  {
    // download.spec.ts "DO-028 should not fail explicitly to cancel a download even if that is
    // already finished".
    name: 'cancel() after a download finished resolves and leaves it finished, file and all',
    path: '/download.html',
    source: `const read = (file) => page.evaluate(async (file) => {
      const response = await fetch('/file?path=' + encodeURIComponent(file))
      return { status: response.status, text: await response.text() }
    }, file)
    const [download] = await Promise.all([page.waitForEvent('download'), page.click('#data')])
    const file = await download.path()
    const canceled = await settle(download.cancel())
    const failure = await download.failure()
    const kept = await read(file)
    await download.delete()
    return { canceled, failure, kept, afterDelete: await read(file) }`,
    expected: {
      canceled: 'undefined',
      failure: null,
      kept: { status: 200, text: 'hello data' },
      afterDelete: { status: 404, text: '' },
    },
    probe: 'location.pathname',
    effect: '/download.html',
  },
]

test('cancel() ends a download the same way through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/download.html': fixture('download.html'),
      '/slow-download': slowDownload,
      '/file': fileContent,
    },
    cases,
  })
})
