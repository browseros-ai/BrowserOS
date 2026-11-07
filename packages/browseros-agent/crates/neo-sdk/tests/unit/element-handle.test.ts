/**
 * `ElementHandle`'s reads against a fake `__neo_runtime` (fake-runtime.ts): each sends its
 * `ElementHandle` command to the handle's guid with Playwright's `kNoTimeout`, as Playwright's
 * client does (`client/elementHandle.ts` at 26137face). An absent value reads `null` where
 * Playwright's does, and a frame answer is the run's own `Frame` object.
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
