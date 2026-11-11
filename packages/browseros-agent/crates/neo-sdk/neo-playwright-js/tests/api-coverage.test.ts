// Every member real playwright-core 1.64.1 defines (tests/data/api-methods.json, recorded by
// scripts/capture-goldens.mjs) exists on the prelude's classes: implemented, or listed in the
// one UNSUPPORTED table with a hint. A Playwright upgrade that adds a member fails here until it
// is classified, and a member Playwright lacks cannot slip into the script-visible surface.
import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { UNSUPPORTED } from '../src/prelude/unsupported'
import { loadPrelude, SEED } from './support/prelude-runtime'

type Members = { methods: string[]; properties: string[] }
type Api = {
  playwright: string
  classes: Record<string, Members> & {
    expect: { statics: string[]; matchers: string[]; modifiers: string[] }
  }
}

const api: Api = JSON.parse(
  readFileSync(new URL('./data/api-methods.json', import.meta.url), 'utf8'),
)

const ELEMENT = {
  $neo: 'element',
  page: { $neo: 'page', tab: 1007 },
  seq: 0,
  node: 5,
  display: '<body></body>',
}

async function instances(): Promise<Record<string, object>> {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => (call.name === 'locator.elementHandle' ? ELEMENT : null),
  })
  return (await runtime.run(`return {
    Selectors: playwright.selectors,
    Browser: browser,
    BrowserContext: context,
    Page: page,
    Frame: page.mainFrame(),
    Locator: page.locator('body'),
    FrameLocator: page.frameLocator('iframe'),
    Keyboard: page.keyboard,
    Mouse: page.mouse,
    ElementHandle: await page.$('body'),
    expect,
    assertion: expect(page.locator('body')),
  }`)) as Record<string, object>
}

/** Public members as playwright-core's capture reads them: own and inherited, no `_` names. */
function members(instance: object): Members {
  const methods = new Set<string>()
  const properties = new Set<string>()
  for (
    let holder: object | null = instance;
    holder && Object.getPrototypeOf(holder) !== null;
    holder = Object.getPrototypeOf(holder)
  ) {
    for (const name of Object.getOwnPropertyNames(holder)) {
      if (name === 'constructor' || name.startsWith('_')) continue
      const descriptor = Object.getOwnPropertyDescriptor(holder, name)
      if (typeof descriptor?.value === 'function') methods.add(name)
      else properties.add(name)
    }
  }
  for (const name of methods) properties.delete(name)
  return { methods: [...methods].sort(), properties: [...properties].sort() }
}

describe('the classes', () => {
  for (const [className, golden] of Object.entries(api.classes)) {
    if (className === 'expect') continue
    it(`tests that ${className} has exactly playwright-core ${api.playwright}'s members`, async () => {
      const instance = (await instances())[className]
      if (!instance) throw new Error(`no ${className} instance`)
      expect(members(instance)).toEqual({
        methods: [...(golden as Members).methods].sort(),
        properties: [...(golden as Members).properties].sort(),
      })
    })
  }

  it('tests that every unsupported entry names a member Playwright defines', () => {
    const stray: string[] = []
    for (const [className, entries] of Object.entries(UNSUPPORTED)) {
      const golden = api.classes[className]
      if (!golden) continue
      const known =
        className === 'expect'
          ? [...api.classes.expect.statics, ...api.classes.expect.matchers]
          : [...(golden as Members).methods, ...(golden as Members).properties]
      for (const member of Object.keys(entries))
        if (!known.includes(member)) stray.push(`${className}.${member}`)
    }
    expect(stray).toEqual([])
  })
})

describe('expect', () => {
  it("tests that expect has playwright's statics, matchers and modifiers", async () => {
    const { expect: expectFn, assertion } = await instances()
    const has = (target: object | undefined, name: string) =>
      target !== undefined && name in target
    expect(
      api.classes.expect.statics.filter((name) => !has(expectFn, name)),
    ).toEqual([])
    expect(
      api.classes.expect.matchers.filter(
        (name) => typeof Reflect.get(Object(assertion), name) !== 'function',
      ),
    ).toEqual([])
    expect(
      api.classes.expect.modifiers.filter((name) => !has(assertion, name)),
    ).toEqual([])
  })
})

describe('unsupported members', () => {
  const cases = Object.entries(UNSUPPORTED).flatMap(([className, entries]) =>
    Object.entries(
      entries as Record<string, string | { hint: string; kind: string }>,
    ).map(([member, entry]) => ({
      className,
      member,
      hint: typeof entry === 'string' ? entry : entry.hint,
      kind: typeof entry === 'string' ? 'async' : entry.kind,
    })),
  )

  it('tests that each fails script / method_unavailable with its hint, one refusal the door records', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'locator.elementHandle'
          ? ELEMENT
          : call.name === 'events.wait' && call.input.event === 'console'
            ? {
                seq: 1,
                event: 'console',
                page: { $neo: 'page', tab: 1007 },
                message: {
                  type: 'log',
                  text: 'x',
                  location: { url: '', lineNumber: 0, columnNumber: 0 },
                  timestamp: 0,
                },
              }
            : call.name === 'events.wait'
              ? {
                  seq: 2,
                  event: 'download',
                  page: { $neo: 'page', tab: 1007 },
                  download: {
                    id: 'guid-1',
                    url: 'https://a.example/f',
                    suggestedFilename: 'f',
                  },
                }
              : call.name === 'page.goto'
                ? {
                    response: {
                      url: 'https://a.example/',
                      status: 200,
                      statusText: 'OK',
                      ok: true,
                      headers: {},
                    },
                    frames: [],
                  }
                : null,
    })
    const outcomes = (await runtime.run(`
      const targets = {
        Selectors: playwright.selectors,
        Browser: browser,
        BrowserContext: context,
        Page: page,
        Frame: page.mainFrame(),
        Locator: page.locator('body'),
        FrameLocator: page.frameLocator('iframe'),
        Keyboard: page.keyboard,
        Mouse: page.mouse,
        ElementHandle: await page.$('body'),
        ConsoleMessage: await page.waitForEvent('console'),
        Download: await page.waitForEvent('download'),
        Response: await page.goto('https://a.example/'),
        expect,
        assertion: expect(page.locator('body')),
      }
      const cases = ${JSON.stringify(cases)}
      const outcomes = []
      for (const { className, member, kind } of cases) {
        const target = className === 'expect' ? (member.startsWith('to') ? targets.assertion : targets.expect) : targets[className]
        let outcome
        try {
          const value = kind === 'property' ? target[member] : target[member]()
          if (kind === 'sync') outcome = { returned: true }
          else if (kind === 'async') outcome = await value.then(() => ({ resolved: true }), (e) => ({ code: e.code, message: e.message, async: true }))
          else outcome = { returned: true }
        } catch (e) {
          outcome = { code: e.code, message: e.message, async: false }
        }
        outcomes.push({ className, member, ...outcome })
      }
      return outcomes
    `)) as {
      className: string
      member: string
      code?: string
      message?: string
      async?: boolean
    }[]
    const api = (className: string, member: string) =>
      `${className.charAt(0).toLowerCase()}${className.slice(1)}.${member}`
    const expected = cases.map(({ className, member, hint, kind }) => ({
      className,
      member,
      code: 'method_unavailable',
      message: `${api(className, member)}: ${hint}`,
      async: kind === 'async',
    }))
    expect(outcomes).toEqual(expected)
    const setup = ['locator.elementHandle', 'events.wait', 'page.goto']
    expect(runtime.calls.filter((call) => !setup.includes(call.name))).toEqual(
      cases.map(({ className, member, hint }) => ({
        name: api(className, member),
        input: {
          $neo: 'refused',
          code: 'method_unavailable',
          reason: `${api(className, member)}: ${hint}`,
        },
      })),
    )
  })
})
