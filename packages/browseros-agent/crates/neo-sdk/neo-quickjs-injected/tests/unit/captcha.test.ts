/** Neo: Pins the host extension wire shape and argument refusal before native admission. */
import { beforeEach, expect, test } from 'bun:test'
import { createRun } from '../../src/index'
import type { NeoCaptchaOutcome } from '../../src/neo-commands'
import { eventless } from './bridge-stub'

let messages: Record<string, unknown>[] = []
const result: NeoCaptchaOutcome = {
  verdict: 'already_clear',
  type: null,
  reason: 'no_captcha',
  message: 'clear',
  durationMs: 12,
  paidTasks: 0,
}
beforeEach(() => {
  messages = []
  ;(globalThis as Record<string, unknown>).__neo_runtime = eventless({
    get: () => 'null',
    call: async (message: string) => {
      const parsed = JSON.parse(message)
      messages.push(parsed)
      return JSON.stringify({ id: parsed.id, result })
    },
  })
})
function run() {
  return createRun({
    page: 'page@T',
    mainFrame: 'frame@T',
    context: 'context@default',
    browser: 'browser@default',
    url: 'about:blank',
    testIdAttribute: 'data-testid',
    lineOffset: 0,
  })
}

test('captcha uses its main frame, empty params and the run deadline', async () => {
  const { page, globals } = run()
  expect(Object.isFrozen(globals.neo)).toBe(true)
  expect(await globals.neo.solveCaptcha(page)).toEqual(result)
  expect(messages).toHaveLength(1)
  expect(messages[0]).toMatchObject({
    guid: 'frame@T',
    method: 'neo:solveCaptcha',
    params: {},
    metadata: { timeout: 0 },
  })
})
test('captcha rejects a non-Page without a native call', () => {
  const { globals } = run()
  // Exercise the untyped script boundary, which TypeScript callers cannot pass accidentally.
  // @ts-expect-error The untyped agent boundary must refuse values other than a Page.
  expect(() => globals.neo.solveCaptcha({})).toThrow(TypeError)
  expect(messages).toEqual([])
})
