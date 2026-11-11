/**
 * Binds Playwright Test's import seam to the active run's public expect parameter.
 * Neo owns matcher execution, retries, options and errors; Playwright keeps value assertions
 * and test-runner statics. The provider is lazy because specs load before worker fixtures.
 */
// The generated Playwright config and the run must use the same assertion deadline.
export const EXPECT_TIMEOUT = 10000

export function productExpect(
  original,
  { isNeo, provider, info },
  config = { timeout: EXPECT_TIMEOUT },
) {
  return new Proxy(original, {
    apply(target, thisArg, args) {
      if (!isNeo(args[0])) return Reflect.apply(target, thisArg, args)
      info()?.annotations.push({ type: 'neoExpect', description: 'product' })
      return provider().configure(config)(...args)
    },
    get(target, name, receiver) {
      // Configured and extended imports must retain the same run binding. Returning the
      // original statics here would silently send subsequent Neo assertions to Playwright.
      if (name === 'configure')
        return (options) =>
          productExpect(
            target.configure(options),
            { isNeo, provider, info },
            { ...config, ...options },
          )
      if (name === 'extend')
        return (matchers) =>
          productExpect(
            target.extend(matchers),
            { isNeo, provider, info },
            config,
          )
      if (name === 'soft')
        return productExpect(
          target.soft,
          { isNeo, provider, info },
          { ...config, soft: true },
        )
      return Reflect.get(target, name, receiver)
    },
  })
}
