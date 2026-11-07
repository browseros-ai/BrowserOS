/**
 * What `JSON.stringify` makes of a run's objects, and so what a script that returns one hands back:
 * Playwright's `ChannelOwner.toJSON` (`client/channelOwner.ts` at 26137face), the object's type and
 * guid only. Neo's guids are its own (`page@<targetId>`, `context@default`), never Playwright's.
 */

import { expect, test } from 'bun:test'
import { OTHER_TAB, RUN_TAB, start } from './fake-runtime'

test('tests that a page, its main frame and its context serialize as their type and guid', () => {
  const { page, context } = start()
  expect(JSON.stringify(page)).toBe('{"_type":"Page","_guid":"page@RUN"}')
  expect(JSON.stringify(page.mainFrame())).toBe(
    '{"_type":"Frame","_guid":"frame@RUN"}',
  )
  expect(JSON.stringify(context)).toBe(
    '{"_type":"BrowserContext","_guid":"context@default"}',
  )
})

test('tests that nested pages serialize the same way, and serializing reads nothing from the tab', () => {
  const { page, context, fake } = start([RUN_TAB, OTHER_TAB])
  const pages = context.pages()
  const reads = fake.gets.length
  expect(
    JSON.parse(JSON.stringify({ pages, frame: pages[1]?.mainFrame() })),
  ).toEqual({
    pages: [
      { _type: 'Page', _guid: 'page@RUN' },
      { _type: 'Page', _guid: 'page@B' },
    ],
    frame: { _type: 'Frame', _guid: 'frame@B' },
  })
  expect(pages[0]).toBe(page)
  expect(fake.gets.length).toBe(reads)
  expect(fake.messages).toEqual([])
})
