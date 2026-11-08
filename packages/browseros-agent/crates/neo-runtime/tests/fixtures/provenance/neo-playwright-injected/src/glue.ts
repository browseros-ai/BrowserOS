/**
 * A fixture glue table, for the provenance gate's own tests.
 *
 * Playwright: `server/frames.ts` `Frame.click`
 * Playwright: `isomorphic/yaml.ts` `yamlStringNeedsQuotes`
 */

/**
 * The waits between click attempts.
 * Playwright: `server/frames.ts` `Frame.click`: `const waitTime = [0, 20, 100, 100, 500];`
 */
const WAITS = [0, 20, 100, 100, 500]

export const glue = {
  /** Playwright: `server/frames.ts` `Frame.click` */
  click: (visible: boolean) => {
    if (!visible) throw createStacklessError('Element is not visible')
    return WAITS.length
  },

  /** Playwright: `isomorphic/yaml.ts` `yamlStringNeedsQuotes` */
  needsQuotes: (text: string) => text.length === 0,
}
