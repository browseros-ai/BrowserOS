/**
 * Binds Playwright Test's import seam to the active run's public expect parameter.
 * Neo owns web matchers; Playwright Test owns value assertions, visual baselines, soft
 * assertions and the matchers a spec defines itself with `expect.extend`.
 * Providers are lazy because specs load before worker fixtures.
 *
 *   expect(x)  ── x is not Neo's ──▶ Playwright Test, unchanged
 *       │
 *       └─ x is a Neo page/locator: one `neoExpect` annotation, then by matcher name
 *            toHaveScreenshot ───────────▶ Playwright Test on a visual receiver (Neo captures)
 *            a spec's own extend matcher ─▶ Playwright Test on x itself (its body calls Neo)
 *            anything else ──────────────▶ Neo's expect, always hard; under `soft` the
 *                                           harness records Neo's failure on the test
 */
// The generated Playwright config and the run must use the same assertion deadline.
export const EXPECT_TIMEOUT = 10000

/** Annotation descriptions, one per owner of the assertion. */
const PRODUCT = 'product'
const VISUAL = 'playwright-visual/neo-screenshot'
const CUSTOM = 'playwright-custom/neo-calls'

export function productExpect(
  original,
  { isNeo, provider, info, visual, custom = new Set() },
  config = { timeout: EXPECT_TIMEOUT },
) {
  const bindings = { isNeo, provider, info, visual, custom }
  return new Proxy(original, {
    apply(target, thisArg, args) {
      if (!isNeo(args[0])) return Reflect.apply(target, thisArg, args)
      const annotation = { type: 'neoExpect', description: PRODUCT }
      info()?.annotations.push(annotation)
      // Neo's expect refuses `soft` by design: soft is the test runner's to own.
      const { soft, ...neoConfig } = config
      const neo = (matchers) => (soft ? softly(matchers, info) : matchers)
      if (!visual && custom.size === 0)
        return neo(provider().configure(neoConfig)(...args))
      let product
      const owned = new Map()
      // Visual assertions never configure Neo: the runner executes its own (soft) screenshot
      // matcher against Neo's captures, whatever Neo's expect would say.
      if (visual)
        owned.set('toHaveScreenshot', {
          description: VISUAL,
          runner: () =>
            Reflect.apply(target, thisArg, [visual(args[0]), ...args.slice(1)]),
        })
      // A spec's own matcher is user code, which Playwright runs with its matcher context; its
      // browser reads are Neo calls on the receiver. `target` is the extended expect, so its
      // configuration and soft mode hold for these.
      for (const name of custom)
        owned.set(name, {
          description: CUSTOM,
          runner: () => Reflect.apply(target, thisArg, args),
        })
      const configured = () => {
        product ??= provider().configure(neoConfig)(...args)
        return neo(product)
      }
      return routedMatchers(configured, owned, annotation)
    },
    get(target, name, receiver) {
      // Configured and extended imports must retain the same run binding. Returning the
      // original statics here would silently send subsequent Neo assertions to Playwright.
      if (name === 'configure')
        return (options) =>
          productExpect(target.configure(options), bindings, {
            ...config,
            ...options,
          })
      // Only the names a spec passes to extend leave Neo, never a built-in matcher Neo lacks:
      // routing those to Playwright would count Playwright's passes as Neo's.
      if (name === 'extend')
        return (matchers) =>
          productExpect(
            target.extend(matchers),
            {
              ...bindings,
              custom: new Set([...custom, ...Object.keys(matchers)]),
            },
            config,
          )
      if (name === 'soft')
        return productExpect(target.soft, bindings, { ...config, soft: true })
      return Reflect.get(target, name, receiver)
    },
  })
}

// Baseline paths, TestInfo, tolerances, attachments and a spec's own matchers belong to the
// test runner. Keep its matcher chain intact for the names it owns, and Neo's for the rest.
function routedMatchers(product, owned, annotation) {
  return new Proxy(
    {},
    {
      get(_target, name) {
        const entry = owned.get(name)
        if (entry)
          return (...args) => {
            annotation.description = entry.description
            return entry.runner()[name](...args)
          }
        if (['not', 'resolves', 'rejects'].includes(name))
          return routedMatchers(
            () => product()[name],
            new Map(
              [...owned].map(([owner, { description, runner }]) => [
                owner,
                { description, runner: () => runner()[name] },
              ]),
            ),
            annotation,
          )
        return Reflect.get(product(), name)
      },
    },
  )
}

/**
 * Neo's hard matchers, made soft as Playwright Test makes its own: a failure is recorded on the
 * running test, which fails at its end, and the script continues. Recording goes through the
 * test's `_failWithError`, the call Playwright's soft step makes (worker/testInfo.ts at
 * v1.64.1); without a test to record on, the failure is thrown, never dropped.
 */
function softly(matchers, info) {
  const record = (error) => {
    const test = info()
    if (typeof test?._failWithError !== 'function') throw error
    test._failWithError(error)
  }
  return new Proxy(matchers, {
    get(target, name) {
      const value = Reflect.get(target, name)
      if (['not', 'resolves', 'rejects'].includes(name))
        return softly(value, info)
      if (typeof value !== 'function') return value
      return (...args) => {
        let result
        try {
          result = Reflect.apply(value, target, args)
        } catch (error) {
          record(error)
          return undefined
        }
        return typeof result?.then === 'function'
          ? result.then(() => undefined, record)
          : result
      }
    },
  })
}
