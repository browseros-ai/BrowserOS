// The `playwright` parameter: Playwright's selectors and error classes as a script reaches them.
import { describe, expect, it } from 'bun:test'
import { json, loadPrelude, SEED } from './support/prelude-runtime'

describe('playwright', () => {
  it('tests that setTestIdAttribute changes what every later getByTestId builds, for the run', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const selectors = json(
      await runtime.run(`
        const before = page.getByTestId('Hello')._selector
        playwright.selectors.setTestIdAttribute('data-my-custom-testid')
        const page_ = page.getByTestId('Hello')._selector
        const frame = page.mainFrame().getByTestId('Hello')._selector
        const nested = page.locator('div').getByTestId('Hello')._selector
        playwright.selectors.setTestIdAttribute('data-pw,data-ti')
        const any = page.getByTestId('Hello')._selector
        return { before, page: page_, frame, nested, any }
      `),
    )
    expect(selectors).toEqual({
      before: 'internal:testid=[data-testid="Hello"s]',
      page: 'internal:testid=[data-my-custom-testid="Hello"s]',
      frame: 'internal:testid=[data-my-custom-testid="Hello"s]',
      nested: 'div >> internal:testid=[data-my-custom-testid="Hello"s]',
      any: 'internal:testid=["data-pw,data-ti"="Hello"s]',
    })
    // A fresh run starts at upstream's default again.
    expect(
      await loadPrelude({ seed: SEED }).run(
        "return page.getByTestId('Hello')._selector",
      ),
    ).toBe('internal:testid=[data-testid="Hello"s]')
  })

  it("tests that errors.TimeoutError is the class of Neo's timeouts, and register is refused", async () => {
    const runtime = loadPrelude({ seed: SEED })
    const result = json(
      await runtime.run(`
        const timeout = await page.waitForTimeout(0).then(() => null)
        let refused
        try { await playwright.selectors.register('tag', () => ({})) } catch (e) { refused = { name: e.name, message: e.message, code: e.code } }
        return {
          same: playwright.errors.TimeoutError === TimeoutError,
          refused,
          timeout,
        }
      `),
    )
    expect(result).toEqual({
      same: true,
      refused: {
        name: 'Error',
        message:
          'selectors.register: custom selector engines are not available in Neo; locate with CSS, text, role or test-id selectors',
        code: 'method_unavailable',
      },
      timeout: null,
    })
  })
})
