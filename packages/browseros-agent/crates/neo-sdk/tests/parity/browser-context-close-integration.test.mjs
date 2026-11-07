// Review: created-context close and default-context disconnect retain distinct lifetimes.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { fixturePage, outcomeValue, withPlatform } from './platform.mjs'

const pages = { '/emulation.html': fixturePage('emulation.html') }
for (const { name, source, expected } of [
  {
    name: 'closing a created context preserves the run context and its sibling',
    source: `const browser=context.browser()
const left=await browser.newContext()
const right=await browser.newContext()
const leftPage=await left.newPage()
const rightPage=await right.newPage()
await left.close({reason:'left closed'})
const result={connected:browser.isConnected(),contexts:browser.contexts().length,
closed:[context.isClosed(),page.isClosed(),left.isClosed(),leftPage.isClosed(),right.isClosed(),rightPage.isClosed()],
ownTitle:await page.title(),siblingTitle:await rightPage.title()}
await right.close()
return result`,
    expected: {
      connected: true,
      contexts: 2,
      closed: [false, false, true, true, false, false],
      ownTitle: 'emulation',
      siblingTitle: '',
    },
  },
  {
    name: 'closing the run context disconnects the browser and closes its created-context objects',
    source: `const browser=context.browser()
const created=await browser.newContext()
const other=await created.newPage()
await context.close({reason:'run disconnected'})
return {connected:browser.isConnected(),contexts:browser.contexts().length,
closed:[context.isClosed(),page.isClosed(),created.isClosed(),other.isClosed()],
pages:[context.pages().length,created.pages().length]}`,
    expected: {
      connected: false,
      contexts: 0,
      closed: [true, true, true, true],
      pages: [0, 0],
    },
  },
]) {
  test(`review F with E2: ${name}`, { skip }, async (t) => {
    await withPlatform(t, { pages }, async ({ playwright, neo, cdp }) => {
      const reference = outcomeValue(
        await playwright('/emulation.html', source),
      )
      assert.deepEqual(reference, expected)
      const result = await neo('/emulation.html', source)
      const actual = outcomeValue(result)
      t.diagnostic(JSON.stringify({ reference, actual }))
      const { targetInfos } = await cdp.send('Target.getTargets')
      assert.ok(
        targetInfos.some((target) => target.targetId === result.targetId),
        'the run context leaves its real tab open',
      )
      const { browserContextIds } = await cdp.send('Target.getBrowserContexts')
      assert.deepEqual(browserContextIds, [], 'created contexts are disposed')
      assert.deepEqual(actual, reference)
    })
  })
}
