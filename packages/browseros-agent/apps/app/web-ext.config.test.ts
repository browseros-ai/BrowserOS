import { afterEach, describe, expect, it } from 'bun:test'

// biome-ignore lint/style/noProcessEnv: test controls import-time env reads.
const env = process.env
const originalProduct = env.BROWSEROS_PRODUCT
let importVersion = 0

afterEach(() => {
  if (originalProduct === undefined) {
    delete env.BROWSEROS_PRODUCT
  } else {
    env.BROWSEROS_PRODUCT = originalProduct
  }
})

async function loadWebExtConfig(product?: string) {
  if (product === undefined) {
    delete env.BROWSEROS_PRODUCT
  } else {
    env.BROWSEROS_PRODUCT = product
  }

  const module = await import(`./web-ext.config.ts?test=${importVersion++}`)
  return module.default as { chromiumArgs: string[] }
}

describe('web-ext Chromium product args', () => {
  it('defaults BrowserOS launches to the BrowserOS product', async () => {
    const config = await loadWebExtConfig()

    expect(config.chromiumArgs).toContain('--browseros-product=browseros')
  })

  it('honors an explicit BrowserOS product override', async () => {
    const config = await loadWebExtConfig('browserclaw')

    expect(config.chromiumArgs).toContain('--browseros-product=browserclaw')
  })

  it('rejects invalid product overrides', async () => {
    await expect(loadWebExtConfig('invalid')).rejects.toThrow(
      'BROWSEROS_PRODUCT must be browseros or browserclaw: invalid',
    )
  })
})
