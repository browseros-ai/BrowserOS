/**
 * Binds Playwright Test's import seam to the active run's public expect parameter.
 * Neo owns web matchers; Playwright Test owns value assertions and visual baselines.
 * Providers are lazy because specs load before worker fixtures.
 */
// The generated Playwright config and the run must use the same assertion deadline.
export const EXPECT_TIMEOUT = 10000

export function productExpect(
  original,
  { isNeo, provider, info, visual },
  config = { timeout: EXPECT_TIMEOUT },
) {
  return new Proxy(original, {
    apply(target, thisArg, args) {
      if (!isNeo(args[0])) return Reflect.apply(target, thisArg, args)
      const annotation = { type: 'neoExpect', description: 'product' }
      info()?.annotations.push(annotation)
      if (!visual) return provider().configure(config)(...args)
      let product
      return visualMatchers(
        // Visual assertions never configure Neo: its standalone soft refusal must not
        // prevent the runner from executing its own soft screenshot matcher.
        () => (product ??= provider().configure(config)(...args)),
        () =>
          Reflect.apply(target, thisArg, [visual(args[0]), ...args.slice(1)]),
        annotation,
      )
    },
    get(target, name, receiver) {
      // Configured and extended imports must retain the same run binding. Returning the
      // original statics here would silently send subsequent Neo assertions to Playwright.
      if (name === 'configure')
        return (options) =>
          productExpect(
            target.configure(options),
            { isNeo, provider, info, visual },
            { ...config, ...options },
          )
      if (name === 'extend')
        return (matchers) =>
          productExpect(
            target.extend(matchers),
            { isNeo, provider, info, visual },
            config,
          )
      if (name === 'soft')
        return productExpect(
          target.soft,
          { isNeo, provider, info, visual },
          { ...config, soft: true },
        )
      return Reflect.get(target, name, receiver)
    },
  })
}

// Baseline paths, TestInfo, tolerances and attachments belong to the test runner. Keep its
// matcher chain intact, while the visual receiver directs all browser reads to Neo.
function visualMatchers(product, runner, annotation) {
  return new Proxy(
    {},
    {
      get(_target, name) {
        if (name === 'toHaveScreenshot')
          return (...args) => {
            annotation.description = 'playwright-visual/neo-capture'
            return runner().toHaveScreenshot(...args)
          }
        if (['not', 'resolves', 'rejects'].includes(name))
          return visualMatchers(
            () => product()[name],
            () => runner()[name],
            annotation,
          )
        return Reflect.get(product(), name)
      },
    },
  )
}
