/**
 * Ungated checks for the fixture server itself — these run anywhere,
 * with or without a browser binary.
 */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import {
  type FixturePair,
  listFixturePages,
  startFixturePair,
} from '../fixtures/server'

const EXPECTED_PAGES = [
  'console.html',
  'cursor.html',
  'dialog.html',
  'dynamic.html',
  'form.html',
  'iframe-child.html',
  'iframe-grandchild.html',
  'iframe.html',
  'injection.html',
  'links.html',
  'media.html',
  'overlay.html',
  'scroll.html',
  'upload.html',
]

describe('fixture server', () => {
  let pair: FixturePair

  beforeAll(async () => {
    pair = await startFixturePair()
  })

  afterAll(async () => {
    await pair.stop()
  })

  test('serves every fixture page on two distinct unrestricted ports', async () => {
    expect(await listFixturePages()).toEqual(EXPECTED_PAGES)
    for (const server of [pair.primary, pair.secondary]) {
      expect(server.port).toBeGreaterThanOrEqual(10101)
      expect(server.port).toBeLessThanOrEqual(20202)
      for (const page of EXPECTED_PAGES) {
        const response = await fetch(server.url(`/${page}`))
        expect(response.status).toBe(200)
        expect(response.headers.get('content-type')).toStartWith('text/html')
      }
    }
    expect(pair.primary.port).not.toBe(pair.secondary.port)
  })

  test('serves the download payload and 404s unknown paths', async () => {
    const download = await fetch(pair.primary.url('/files/report.txt'))
    expect(download.status).toBe(200)
    expect(download.headers.get('content-disposition')).toContain('report.txt')
    expect(await download.text()).toContain('fixture report contents')

    expect((await fetch(pair.primary.url('/nope.html'))).status).toBe(404)
    expect((await fetch(pair.primary.url('/etc/passwd'))).status).toBe(404)
  })

  test('fixture pages carry the hooks the case matrix relies on', async () => {
    const read = async (page: string) =>
      await (await fetch(pair.primary.url(`/${page}`))).text()

    expect(await read('form.html')).toInclude('id="result"')
    expect(await read('form.html')).toInclude('Disabled action')
    expect(await read('links.html')).toInclude('download="report.txt"')
    expect(await read('iframe.html')).toInclude('childOrigin')
    const cursor = await read('cursor.html')
    expect(cursor).toInclude('contenteditable')
    expect(cursor).not.toInclude('aria-')
    expect(cursor).not.toInclude('role=')
    const dynamic = await read('dynamic.html')
    expect(dynamic).toInclude('delayed content ready')
    expect(dynamic).toInclude('3000')
    expect(await read('overlay.html')).toInclude('Overlay cover')
    expect(await read('dialog.html')).toInclude('fixture confirm')
    const scroll = await read('scroll.html')
    expect(scroll).toInclude('scroll-pos')
    expect(scroll).toInclude('drag-order')
    expect(scroll).toInclude('tooltip revealed')
    expect(await read('upload.html')).toInclude('multiple')
    expect(await read('console.html')).toInclude('fixture log one')
    const injection = await read('injection.html')
    expect(injection).toInclude('IGNORE PREVIOUS INSTRUCTIONS')
    expect(injection).toInclude('[ref=e99]')
    expect(injection).toInclude('[END_UNTRUSTED_PAGE_CONTENT nonce=')
    expect(await read('media.html')).toInclude('block blue')
  })
})
