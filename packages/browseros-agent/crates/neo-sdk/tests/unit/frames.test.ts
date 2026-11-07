/** Live frame getters and Rust-minted identities against the synchronous bridge contract. */
import { describe, expect, test } from 'bun:test'
import urlGoldens from '../../../neo-runtime/tests/fixtures/url-match-goldens.json'
import type { CreatedFrame } from '../../src/runtime-bridge'
import { failure, start } from './fake-runtime'

const MAIN = 'frame@RUN'
const LOGIN = 'opaque login frame'
const NESTED = 'opaque nested frame'
const PAY = 'opaque payment frame'

function record(
  guid: string,
  name: string,
  url: string,
  parent?: string,
): CreatedFrame {
  return {
    guid,
    initializer: {
      name,
      url,
      ...(parent ? { parentFrame: { guid: parent } } : {}),
      loadStates: [],
    },
  }
}

function startFrames(
  initial?: CreatedFrame[],
  afterFramesRead?: (records: Map<string, CreatedFrame>) => void,
) {
  const run = start()
  const records = new Map(
    (
      initial ?? [
        record(MAIN, '', 'https://run.test/'),
        record(LOGIN, 'login', 'https://login.test/', MAIN),
        record(NESTED, 'nested', 'https://nested.test/', LOGIN),
        record(PAY, 'pay', 'https://pay.test/', MAIN),
      ]
    ).map((frame) => [frame.guid, frame]),
  )
  Object.assign(globalThis, {
    __neo_runtime: {
      call: run.fake.call.bind(run.fake),
      get(request: string) {
        const { guid, field, url, matcher } = JSON.parse(request)
        if (field === 'matchesURL') {
          const golden =
            'glob' in matcher
              ? urlGoldens.globs.find((entry) => entry.glob === matcher.glob)
              : urlGoldens.regexes.find(
                  (entry) =>
                    entry.source === matcher.regexSource &&
                    entry.flags === matcher.regexFlags,
                )
          if (golden && 'error' in golden) throw new Error(golden.error)
          const match =
            golden && 'urls' in golden
              ? golden.urls.find(([value]) => value === url)
              : undefined
          if (!match)
            throw new Error(
              'URL matcher test needs a recorded Playwright result',
            )
          return JSON.stringify(match[1])
        }
        if (guid === 'page@RUN' && field === 'frames') {
          const reply = JSON.stringify([...records.values()])
          afterFramesRead?.(records)
          return reply
        }
        if (field === 'isDetached') return JSON.stringify(!records.has(guid))
        const frame = records.get(guid)
        if (!frame) {
          if (field === 'url' || field === 'name' || field === 'parentFrame')
            return JSON.stringify(null)
          if (field === 'childFrames') return JSON.stringify([])
          throw new Error('Frame was detached')
        }
        if (field === 'url') return JSON.stringify(frame.initializer.url)
        if (field === 'name') return JSON.stringify(frame.initializer.name)
        if (field === 'parentFrame')
          return JSON.stringify(
            records.get(frame.initializer.parentFrame?.guid ?? '') ?? null,
          )
        if (field === 'childFrames')
          return JSON.stringify(
            [...records.values()].filter(
              (child) => child.initializer.parentFrame?.guid === guid,
            ),
          )
        return run.fake.get(request)
      },
    },
  })
  return { ...run, records }
}

describe('direct frame identity', () => {
  test('tests that frames, parents and children share one object per guid', () => {
    const { page } = startFrames()
    const [main, login, nested, pay] = page.frames()
    expect(main).toBe(page.mainFrame())
    expect(page.frames()[1]).toBe(login)
    expect(login!.parentFrame()).toBe(main)
    expect(nested!.parentFrame()).toBe(login)
    expect(pay!.parentFrame()).toBe(main)
    expect(main!.parentFrame()).toBeNull()
    expect(main!.childFrames()[0]).toBe(login)
    expect(main!.childFrames()[1]).toBe(pay)
    expect(login!.childFrames()[0]).toBe(nested)
    expect(nested!.childFrames()).toEqual([])
    expect(login!.page()).toBe(page)
    expect(page.frame('login')).toBe(login)
  })

  test('tests that a child found before frames() gets the same identity', () => {
    const { page } = startFrames()
    const login = page.mainFrame().childFrames()[0]!
    const nested = login.childFrames()[0]!
    expect(page.frames()[1]).toBe(login)
    expect(page.frames()[2]).toBe(nested)
    expect(nested.parentFrame()).toBe(login)
  })

  test('tests that names, URLs, tree membership and detach state stay live', () => {
    const { page, records } = startFrames()
    const login = page.frame('login')!
    expect(login.isDetached()).toBe(false)
    expect(login.url()).toBe('https://login.test/')
    records.set(LOGIN, record(LOGIN, 'renamed', 'https://login.test/new', MAIN))
    expect(login.name()).toBe('renamed')
    expect(login.url()).toBe('https://login.test/new')
    expect(page.frame('renamed')).toBe(login)
    records.delete(LOGIN)
    records.delete(NESTED)
    expect(login.isDetached()).toBe(true)
    expect(page.frames()).not.toContain(login)
    expect(page.mainFrame().childFrames()).toHaveLength(1)
    expect(login.url()).toBe('https://login.test/new')
    expect(login.name()).toBe('renamed')
    expect(login.parentFrame()).toBeNull()
    expect(login.childFrames()).toEqual([])
  })

  test('tests that a frame removed after enumeration retains its initializer values', () => {
    const { page, records } = startFrames()
    const [main, login, nested, pay] = page.frames()
    records.clear()
    expect(main!.url()).toBe('https://run.test/')
    expect(main!.name()).toBe('')
    for (const [frame, name, url] of [
      [login!, 'login', 'https://login.test/'],
      [nested!, 'nested', 'https://nested.test/'],
      [pay!, 'pay', 'https://pay.test/'],
    ] as const) {
      expect(frame.isDetached()).toBe(true)
      expect(frame.name()).toBe(name)
      expect(frame.url()).toBe(url)
      expect(frame.parentFrame()).toBeNull()
      expect(frame.childFrames()).toEqual([])
    }
  })

  test('tests that later frame records refresh the values retained after detachment', () => {
    const { page, records } = startFrames()
    const frame = page.frames()[1]!
    records.set(LOGIN, record(LOGIN, 'renamed', 'https://login.test/new', MAIN))
    expect(page.frames()[1]).toBe(frame)
    records.delete(LOGIN)
    expect(frame.name()).toBe('renamed')
    expect(frame.url()).toBe('https://login.test/new')
  })

  test('tests that frame names are exact and take precedence over URL', () => {
    const { page } = startFrames()
    expect(page.frame('login')).toBe(page.frames()[1])
    expect(page.frame({ name: 'pay' })).toBe(page.frames()[3])
    expect(page.frame({ name: 'pay', url: '**/absent' })).toBe(page.frames()[3])
    expect(page.frame('log')).toBeNull()
    expect(page.frame('LOGIN')).toBeNull()
    expect(() => page.frame({})).toThrow(Error)
  })

  test('tests that cross-frame composite locators are rejected', () => {
    const { page } = startFrames()
    const child = page.frame('login')!
    const local = child.locator('input')
    const parent = page.locator('input')
    expect(() => local.locator(parent)).toThrow(Error)
    expect(() => local.and(parent)).toThrow(Error)
    expect(() => local.or(parent)).toThrow(Error)
    expect(() => child.locator('li', { has: parent })).toThrow(Error)
    expect(() => child.locator('li', { hasNot: parent })).toThrow(Error)
    expect(() => child.frameLocator('iframe').locator(parent)).toThrow(Error)
    expect(local.locator(page.frames()[1]!.locator('span')).page()).toBe(page)
  })
})

describe('Page.frame URL matching', () => {
  for (const [kind, selector, missUrl] of [
    ['string', 'target', 'http://a.test/undone.html'],
    ['name', { name: 'target' }, 'http://a.test/undone.html'],
    ['glob', { url: '**/done.html' }, 'http://a.test/undone.html'],
    ['regexp', { url: /done\.html$/ }, 'http://a.test/done.htm'],
  ] as const) {
    test(`tests that ${kind} lookup uses one frame snapshot and returns the canonical frame`, () => {
      const { page } = startFrames(
        [
          record(MAIN, '', missUrl),
          record(PAY, 'target', 'http://a.test/done.html', MAIN),
        ],
        (records) => {
          for (const [guid, frame] of records)
            records.set(guid, {
              ...frame,
              initializer: {
                ...frame.initializer,
                name: 'changed',
                url: missUrl,
              },
            })
        },
      )
      const [target] = page.mainFrame().childFrames()
      expect(target).toBeDefined()
      expect(page.frame(selector)).toBe(target)
      expect(page.frame(selector)).toBeNull()
    })
  }

  for (const golden of urlGoldens.globs) {
    const urls = golden.urls
    if (!urls) continue
    test(`tests that glob ${JSON.stringify(golden.glob)} selects Playwright's first match`, () => {
      const { page } = startFrames(
        urls.map(([url], index) =>
          record(
            index === 0 ? MAIN : `opaque-${index}`,
            `frame-${index}`,
            String(url),
          ),
        ),
      )
      const index = urls.findIndex(([, matches]) => matches)
      if (golden.glob === '') {
        expect(() => page.frame({ url: golden.glob })).toThrow(Error)
        return
      }
      expect(page.frame({ url: golden.glob })).toBe(
        index === -1 ? null : page.frames()[index],
      )
    })
  }
  for (const golden of urlGoldens.regexes) {
    test(`tests that regexp /${golden.source}/${golden.flags} selects Playwright's first match`, () => {
      const { page } = startFrames(
        golden.urls.map(([url], index) =>
          record(
            index === 0 ? MAIN : `opaque-${index}`,
            `frame-${index}`,
            String(url),
          ),
        ),
      )
      const index = golden.urls.findIndex(([, matches]) => matches)
      const matcher = new RegExp(golden.source, golden.flags)
      matcher.lastIndex = 99
      expect(page.frame({ url: matcher })).toBe(
        index === -1 ? null : page.frames()[index],
      )
      expect(page.frame({ url: matcher })).toBe(
        index === -1 ? null : page.frames()[index],
      )
    })
  }
  test('tests that a URL predicate is refused even with no frames', () => {
    const { page } = startFrames([])
    expect(() => page.frame({ url: (() => true) as never })).toThrow(
      'predicate',
    )
  })
})

test('tests that child-frame actions and reads use their own frame', async () => {
  const { page, fake } = startFrames()
  const child = page.frame('login')!
  let value = ''
  let checked = false
  fake.answer = ({ guid, method, params }) => {
    if (guid !== LOGIN) return failure('Error', 'Wrong frame')
    if (method === 'goto') return failure('Error', 'Child goto is unsupported')
    if (method === 'fill') value = (params as { value: string }).value
    if (method === 'check') checked = true
    if (method === 'uncheck') checked = false
    if (method === 'inputValue') return { result: { value } }
    if (method === 'isChecked') return { result: { value: checked } }
    if (method === 'isVisible') return { result: { value: true } }
    if (method === 'textContent' || method === 'innerText')
      return { result: { value: 'child text' } }
    if (method === 'getAttribute')
      return { result: { value: 'child attribute' } }
    if (method === 'title') return { result: { value: 'Child title' } }
    if (method === 'selectOption') return { result: { values: ['child'] } }
    if (method === 'queryCount') return { result: { value: 2 } }
    if (method === 'evalOnSelectorAll')
      return { result: { value: { id: 1, a: ['child text'] } } }
    return {}
  }
  await child.fill('input', 'alice')
  expect(await child.inputValue('input')).toBe('alice')
  await child.getByLabel('Email').fill('bob')
  expect(await child.locator('input').inputValue()).toBe('bob')
  await child.click('button')
  await child.press('input', 'Enter')
  await child.check('input')
  expect(await child.isChecked('input')).toBe(true)
  await child.uncheck('input')
  expect(await child.isChecked('input')).toBe(false)
  await child.setChecked('input', true)
  expect(await child.isChecked('input')).toBe(true)
  expect(await child.isVisible('input')).toBe(true)
  expect(await child.selectOption('select', 'child')).toEqual(['child'])
  expect(await child.textContent('p')).toBe('child text')
  expect(await child.innerText('p')).toBe('child text')
  expect(await child.getAttribute('p', 'data-value')).toBe('child attribute')
  expect(await child.title()).toBe('Child title')
  expect(await child.locator('p').count()).toBe(2)
  expect(await child.locator('p').allTextContents()).toEqual(['child text'])
  await child.locator('p').waitFor()
  await child.waitForURL('**/done')
  await child.waitForLoadState()
  await expect(child.goto('https://other.test/')).rejects.toBeInstanceOf(Error)
})
