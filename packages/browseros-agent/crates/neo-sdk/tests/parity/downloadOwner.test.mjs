// Which page a download belongs to, through the pinned Playwright and through Neo, on fresh
// tests/pages/download-owner.html tabs (events.mjs): a link that opens a new tab to an attachment
// reports the download on the page that opened the tab, which closes before its first document
// (Playwright's `_onDownloadWillBegin` reports a new window's download on its opener); a link in
// a nested same-origin frame, and one in a cross-site frame Chromium runs out of process, report
// it on the page; and a goto that turns into a download rejects while the page reports the
// download and stays where it was.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/library/download.spec.ts "DO-017 should report new window downloads".
    name: "a new window's download reports on the page that opened it",
    path: '/download-owner.html',
    source: `const [download] = await Promise.all([page.waitForEvent('download'), page.click('#blank')])
    return {
      page: download.page() === page,
      download: [path(download.url()), download.suggestedFilename()],
      failure: await download.failure(),
      saved: typeof (await download.path()),
    }`,
    expected: {
      page: true,
      download: ['/attachment', 'report.csv'],
      failure: null,
      saved: 'string',
    },
    probe: 'location.pathname',
    effect: '/download-owner.html',
  },
  {
    // download.spec.ts "DO-030 should emit download event from nested iframes", and a cross-site
    // frame's link.
    name: 'a link in a nested frame or a cross-site frame reports its download on the page',
    path: '/download-owner.html',
    source: `const [nested] = await Promise.all([
      page.waitForEvent('download'),
      page.frameLocator('#same').frameLocator('#inner').locator('#nested').click(),
    ])
    const [cross] = await Promise.all([
      page.waitForEvent('download'),
      page.frameLocator('#cross').locator('#cross-link').click(),
    ])
    return {
      pages: [nested.page() === page, cross.page() === page],
      urls: [nested.url(), cross.url()].map((url) => url.replace(/^http:\\/\\/([^/:]+):\\d+/, '$1')),
      failures: [await nested.failure(), await cross.failure()],
    }`,
    expected: {
      pages: [true, true],
      urls: ['127.0.0.1/attachment', 'localhost/attachment'],
      failures: [null, null],
    },
    probe: 'location.pathname',
    effect: '/download-owner.html',
  },
  {
    // download.spec.ts "DO-001 should report download when navigation turns into download
    // @smoke".
    name: 'a goto that turns into a download rejects, and the page reports the download',
    path: '/download-owner.html',
    source: `const [download, goto] = await Promise.all([
      page.waitForEvent('download'),
      settle(page.goto(page.url().replace(/\\/[^/]*$/, '/attachment'))),
    ])
    return {
      goto,
      page: download.page() === page,
      url: path(download.url()),
      failure: await download.failure(),
      pageUrl: path(page.url()),
    }`,
    expected: {
      goto: { error: 'Error' },
      page: true,
      url: '/attachment',
      failure: null,
      pageUrl: '/download-owner.html',
    },
    probe: 'location.pathname',
    effect: '/download-owner.html',
  },
]

const ATTACHMENT = {
  headers: {
    'content-type': 'application/octet-stream',
    'content-disposition': 'attachment; filename="report.csv"',
  },
  body: 'a,b\n1,2\n',
}

test('downloads report on the same page through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/download-owner.html': fixture('download-owner.html'),
      '/attachment': ATTACHMENT,
      '/owner-outer.html':
        '<!doctype html><title>outer</title><iframe id="inner" src="/owner-inner.html"></iframe>',
      '/owner-inner.html':
        '<!doctype html><title>inner</title><a id="nested" href="/attachment">Nested</a>',
      '/owner-cross.html':
        '<!doctype html><title>cross</title><a id="cross-link" href="/attachment">Cross</a>',
    },
    cases,
  })
})
