/** Frame selector strings recorded from Playwright 26137face, and public frame guards. */
import { describe, expect, test } from 'bun:test'
import goldens from '../goldens/frame-selector-goldens.json'
import { start } from './fake-runtime'

describe('FrameLocator selectors', () => {
  for (const golden of goldens) {
    test(`tests that ${golden.chain} matches Playwright`, () => {
      const { page } = start()
      const built = new Function('page', `return ${golden.chain}`)(page)
      expect('kind' in golden ? built.owner()._selector : built._selector).toBe(
        golden.selector,
      )
    })
  }

  test('tests that frame locators preserve their page and owner', () => {
    const { page } = start()
    const owner = page.locator('#outer').nth(1)
    expect(owner.contentFrame().owner()._equals(owner)).toBe(true)
    expect(owner.contentFrame().getByLabel('Email').page()).toBe(page)
  })

  test('tests that cross-frame locator arguments are rejected', () => {
    const { page, context } = start([
      { id: 'RUN', url: 'https://run.test/' },
      { id: 'OTHER', url: 'https://other.test/' },
    ])
    const other = context.pages()[1]!.locator('input')
    const frame = page.frameLocator('#login')
    expect(() => frame.locator(other)).toThrow(Error)
    expect(() => frame.locator('li', { has: other })).toThrow(Error)
    expect(() => frame.locator('li', { hasNot: other })).toThrow(Error)
  })

  test('tests that any-frame selection refuses first, last and nth', () => {
    const { page } = start()
    const frame = page.frameLocator()
    expect(() => frame.first()).toThrow(Error)
    expect(() => frame.last()).toThrow(Error)
    expect(() => frame.nth(1)).toThrow(Error)
  })
})
