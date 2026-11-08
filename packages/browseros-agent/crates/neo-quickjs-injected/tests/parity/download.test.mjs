// page.on('download') and context.on('download') through the pinned Playwright and through Neo,
// on fresh tests/pages/download.html tabs (events.mjs): a link with a download attribute (a
// data: URL) and a link to an attachment each start a download that reaches the page's listeners
// and then the context's as one Download (client/page.ts's 'download' handler), with its URL,
// suggested name and page; it finishes with no failure, path() names a file holding the
// downloaded bytes, delete() removes that file, and the page stays where it was.
//
// The two engines save the file in different places: Playwright in its artifacts folder under a
// guid, Neo where the browser saves it (the test profile's download folder, harness.mjs
// `launchBrowser`). So a script never returns a path: it reads the file back through its page
// (harness.mjs `fileContent`, which refuses a file outside the temporary directory). Neo
// refuses saveAs() and createReadStream() for now, a case Playwright cannot share.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { fileContent, skip } from './harness.mjs'

/** `read(file)`: how the fixture server answers for the file at `file`, through the page. */
const READ = `const read = (file) => page.evaluate(async (file) => {
      const response = await fetch('/file?path=' + encodeURIComponent(file))
      return { status: response.status, text: await response.text() }
    }, file)
    `

const cases = [
  {
    // tests/library/download.spec.ts "DO-037 should download links with data url", "DO-013
    // should report non-navigation downloads", "DO-018 should delete file";
    // tests/library/browsercontext-events.spec.ts "BC-050 download event should work @smoke".
    name: "a download attribute's download reaches page then context, and its file reads back",
    path: '/download.html',
    source: `${READ}const order = []
    let fromPage
    const onPage = (download) => {
      fromPage = download
      order.push('page')
    }
    const onContext = (download) => order.push(download === fromPage ? 'context' : 'context (another object)')
    page.on('download', onPage)
    page.context().on('download', onContext)
    const [download] = await Promise.all([page.waitForEvent('download'), page.click('#data')])
    page.off('download', onPage)
    page.context().off('download', onContext)
    const file = await download.path()
    const saved = await read(file)
    const failure = await download.failure()
    const deleted = await settle(download.delete())
    return {
      order,
      download: {
        url: download.url(),
        suggestedFilename: download.suggestedFilename(),
        page: download.page() === page,
        waited: download === fromPage,
      },
      failure,
      saved,
      deleted,
      afterDelete: await read(file),
    }`,
    expected: {
      order: ['page', 'context'],
      download: {
        url: 'data:text/plain,hello%20data',
        suggestedFilename: 'data.txt',
        page: true,
        waited: true,
      },
      failure: null,
      saved: { status: 200, text: 'hello data' },
      deleted: 'undefined',
      afterDelete: { status: 404, text: '' },
    },
    probe: 'location.pathname',
    effect: '/download.html',
  },
  {
    // download.spec.ts "DO-004 should report downloads with acceptDownloads: true", "DO-018
    // should delete file"; browsercontext-events.spec.ts "BC-050 download event should work
    // @smoke" (an attachment with a filename).
    name: "an attachment's download has its URL, name and content, and the page stays put",
    path: '/download.html',
    source: `${READ}const [download] = await Promise.all([page.waitForEvent('download'), page.click('#attachment')])
    const file = await download.path()
    const saved = await read(file)
    const failure = await download.failure()
    await download.delete()
    return {
      download: [path(download.url()), download.suggestedFilename(), download.page() === page],
      failure,
      saved,
      afterDelete: await read(file),
      pageUrl: path(page.url()),
    }`,
    expected: {
      download: ['/attachment', 'report.csv', true],
      failure: null,
      saved: { status: 200, text: 'a,b\n1,2\n' },
      afterDelete: { status: 404, text: '' },
      pageUrl: '/download.html',
    },
    probe: 'location.pathname',
    effect: '/download.html',
  },
  {
    // download.spec.ts "DO-007 should save to user-specified path without updating original
    // path" and "DO-019 should expose stream", which Neo refuses: saveAs() has no run artifact
    // folder to save into yet, and QuickJS has no streams. Neo only.
    name: 'Neo refuses saveAs() and createReadStream() on a download',
    path: '/download.html',
    source: `const [download] = await Promise.all([page.waitForEvent('download'), page.click('#data')])
    const refused = {
      saveAs: await settle(download.saveAs('x.txt')),
      createReadStream: await settle(download.createReadStream()),
    }
    await download.delete()
    return refused`,
    neo: (outcome) => {
      assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
      assert.deepEqual(outcome.value, {
        saveAs: { error: 'Error' },
        createReadStream: { error: 'Error' },
      })
    },
  },
]

test('downloads report the same Download with the same file through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/download.html': fixture('download.html'),
      '/attachment': {
        headers: {
          'content-type': 'application/octet-stream',
          'content-disposition': 'attachment; filename="report.csv"',
        },
        body: 'a,b\n1,2\n',
      },
      '/file': fileContent,
    },
    cases,
  })
})
