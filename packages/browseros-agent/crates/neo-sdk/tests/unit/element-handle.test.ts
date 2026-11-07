/**
 * `ElementHandle`'s reads and actions against a fake `__neo_runtime` (fake-runtime.ts): each sends
 * its `ElementHandle` command to the handle's guid, the reads with Playwright's `kNoTimeout` and
 * the actions with the timeout of the handle's page, as Playwright's client does
 * (`client/elementHandle.ts` at 26137face). An absent value reads `null` where Playwright's does,
 * and a frame answer is the run's own `Frame` object.
 */

import { describe, expect, test } from 'bun:test'
import type { ElementHandle } from '../../src/element-handle'
import { NO_TIMEOUT, start } from './fake-runtime'

const ELEMENT = {
  type: 'ElementHandle',
  guid: 'handle@r.1',
  initializer: { preview: 'JSHandle@<p id="x">x</p>' },
}

async function element() {
  const run = start()
  run.fake.answer = () => ({ result: { element: ELEMENT }, log: [] })
  const handle = (await run.page.$('#x')) as ElementHandle
  return { ...run, handle }
}

describe('ElementHandle reads', () => {
  test('send their command to the handle with no timeout', async () => {
    const { fake, handle } = await element()
    const answers: Record<string, object> = {
      textContent: {},
      getAttribute: { value: 'v' },
      innerText: { value: 'text' },
      innerHTML: { value: '<b>x</b>' },
      inputValue: { value: '' },
      isVisible: { value: true },
      isChecked: { value: false },
      boundingBox: { value: { x: 1, y: 2, width: 3, height: 4 } },
    }
    fake.answer = ({ method }) => ({ result: answers[method], log: [] })
    const sent: [string, object][] = [
      ['textContent', {}],
      ['getAttribute', { name: 'data-a' }],
      ['innerText', {}],
      ['innerHTML', {}],
      ['inputValue', {}],
      ['isVisible', {}],
      ['isChecked', {}],
      ['boundingBox', {}],
    ]
    fake.messages.length = 0
    expect(await handle.textContent()).toBeNull()
    expect(await handle.getAttribute('data-a')).toBe('v')
    expect(await handle.innerText()).toBe('text')
    expect(await handle.innerHTML()).toBe('<b>x</b>')
    expect(await handle.inputValue()).toBe('')
    expect(await handle.isVisible()).toBe(true)
    expect(await handle.isChecked()).toBe(false)
    expect(await handle.boundingBox()).toEqual({
      x: 1,
      y: 2,
      width: 3,
      height: 4,
    })
    expect(
      fake.messages.map(({ guid, method, params, metadata }) => ({
        guid,
        method,
        params,
        metadata,
      })),
    ).toEqual(
      sent.map(([method, params]) => ({
        guid: 'handle@r.1',
        method,
        params,
        metadata: NO_TIMEOUT,
      })),
    )
  })

  test('a box the element has not is null', async () => {
    const { fake, handle } = await element()
    fake.answer = () => ({ result: {}, log: [] })
    expect(await handle.boundingBox()).toBeNull()
    expect(await handle.getAttribute('missing')).toBeNull()
  })

  test('ownerFrame and contentFrame answer the run’s frame objects', async () => {
    const { fake, page, handle } = await element()
    const child = {
      guid: `${page.mainFrame()._guid}/child`,
      initializer: { url: 'http://fixture/child', name: 'kid', loadStates: [] },
    }
    let answer: object = {
      frame: {
        guid: page.mainFrame()._guid,
        initializer: { url: '', name: '', loadStates: [] },
      },
    }
    fake.answer = () => ({ result: answer, log: [] })
    expect(await handle.ownerFrame()).toBe(page.mainFrame())
    answer = { frame: child }
    const first = await handle.contentFrame()
    expect(first?._guid).toBe(child.guid)
    expect(first?.page()).toBe(page)
    expect(await handle.contentFrame()).toBe(first)
    answer = {}
    expect(await handle.contentFrame()).toBeNull()
  })
})

describe('ElementHandle actions', () => {
  test('send their command to the handle with its page’s timeout', async () => {
    const run = start()
    run.fake.answer = () => ({
      result: { element: { ...ELEMENT, frame: run.page.mainFrame()._guid } },
      log: [],
    })
    const handle = (await run.page.$('#x')) as ElementHandle
    run.page.setDefaultTimeout(700)
    run.fake.answer = ({ method }) => ({
      result: method === 'selectOption' ? { values: ['b'] } : {},
      log: [],
    })
    run.fake.messages.length = 0
    await handle.click({ button: 'right', noWaitAfter: true, timeout: 5 })
    await handle.dblclick({ delay: 1 })
    await handle.hover({ noWaitAfter: true })
    await handle.tap({ trial: true })
    await handle.check({ force: true, noWaitAfter: true })
    await handle.uncheck()
    await handle.setChecked(true, { timeout: 9 })
    await handle.setChecked(false)
    await handle.fill('v', { force: true })
    await handle.press('Enter', { delay: 2, noWaitAfter: true })
    await handle.type('ab', { delay: 3 })
    expect(await handle.selectOption('b')).toEqual(['b'])
    await handle.selectText({ force: true })
    await handle.focus()
    await handle.scrollIntoViewIfNeeded({ timeout: 4 })
    const sent: [string, object, number][] = [
      ['click', { noWaitAfter: true, button: 'right' }, 5],
      ['dblclick', { delay: 1 }, 700],
      ['hover', {}, 700],
      ['tap', { trial: true }, 700],
      ['check', { force: true }, 700],
      ['uncheck', {}, 700],
      ['check', {}, 9],
      ['uncheck', {}, 700],
      ['fill', { value: 'v', force: true }, 700],
      ['press', { key: 'Enter', delay: 2, noWaitAfter: true }, 700],
      ['type', { text: 'ab', delay: 3 }, 700],
      ['selectOption', { options: [{ valueOrLabel: 'b' }] }, 700],
      ['selectText', { force: true }, 700],
      ['focus', {}, 0],
      ['scrollIntoViewIfNeeded', {}, 4],
    ]
    expect(
      run.fake.messages.map(({ guid, method, params, metadata }) => [
        guid,
        method,
        params,
        (metadata as { timeout?: number }).timeout,
      ]),
    ).toEqual(
      sent.map(([method, params, timeout]) => [
        'handle@r.1',
        method,
        params,
        timeout,
      ]),
    )
  })

  test('a null option fails before anything is sent', async () => {
    const run = start()
    run.fake.answer = () => ({
      result: { element: { ...ELEMENT, frame: run.page.mainFrame()._guid } },
      log: [],
    })
    const handle = (await run.page.$('#x')) as ElementHandle
    run.fake.messages.length = 0
    await expect(handle.click({ button: null as never })).rejects.toThrow(
      'button: expected a value, got null',
    )
    expect(run.fake.messages).toEqual([])
  })

  test('ElementHandle values travel as their guids, as the frame’s selectOption sends them', async () => {
    const run = start()
    run.fake.answer = () => ({
      result: { element: { ...ELEMENT, frame: run.page.mainFrame()._guid } },
      log: [],
    })
    const handle = (await run.page.$('#x')) as ElementHandle
    run.fake.answer = () => ({ result: { values: ['a'] }, log: [] })
    run.fake.messages.length = 0
    expect(await handle.selectOption(handle)).toEqual(['a'])
    expect(
      run.fake.messages.map(({ method, params }) => [method, params]),
    ).toEqual([['selectOption', { elements: [{ guid: 'handle@r.1' }] }]])
  })
})

describe('ElementHandle waits', () => {
  test('send their command with the page’s timeout and answer the found element', async () => {
    const run = start()
    run.fake.answer = () => ({
      result: { element: { ...ELEMENT, frame: run.page.mainFrame()._guid } },
      log: [],
    })
    const handle = (await run.page.$('#x')) as ElementHandle
    run.page.setDefaultTimeout(700)
    const found = {
      type: 'ElementHandle',
      guid: 'handle@r.2',
      initializer: { preview: 'JSHandle@<span></span>' },
    }
    run.fake.answer = ({ method, params }) => ({
      result:
        method === 'waitForSelector' &&
        (params as { state?: string }).state !== 'detached'
          ? { element: found }
          : {},
      log: [],
    })
    run.fake.messages.length = 0
    expect(await handle.waitForElementState('visible')).toBeUndefined()
    const span = await handle.waitForSelector('span', {
      state: 'attached',
      timeout: 5,
    })
    expect(span?._guid).toBe('handle@r.2')
    expect(
      await handle.waitForSelector('span', { state: 'detached' }),
    ).toBeNull()
    expect(
      run.fake.messages.map(({ guid, method, params, metadata }) => [
        guid,
        method,
        params,
        (metadata as { timeout?: number }).timeout,
      ]),
    ).toEqual([
      ['handle@r.1', 'waitForElementState', { state: 'visible' }, 700],
      [
        'handle@r.1',
        'waitForSelector',
        { selector: 'span', state: 'attached' },
        5,
      ],
      [
        'handle@r.1',
        'waitForSelector',
        { selector: 'span', state: 'detached' },
        700,
      ],
    ])
  })
})
