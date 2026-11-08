// playwright.chromium / firefox / webkit through Neo and through Playwright on one BrowserOS
// neo: their names, the browser's own type, and Firefox refusing to connect over CDP.
//
// Playwright cases (pin 26137face): `tests/library/browsertype-basic.spec.ts` "BT-002
// browserType.name should work" and "BT-003 should throw when trying to connect with
// not-chromium".
//
// Deliberate differences, asserted per engine: Neo never launches or connects to a browser (it
// runs attached to BrowserOS), so `launch`, `launchPersistentContext`, `launchServer`, `connect`
// and `connectOverCDP` reject on Neo before any browser starts, and `executablePath()` throws,
// where Playwright answers the path of the build it would launch (`BT-001`). Playwright's launch
// is not run here: it would start an unrelated browser.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const source = `return {
  names: [playwright.chromium.name(), playwright.firefox.name(), playwright.webkit.name()],
  own: context.browser().browserType() === playwright.chromium,
  firefoxOverCDP: await playwright.firefox.connectOverCDP({ endpointURL: 'ws://foo' }).then(() => 'connected', errorCategory),
}`

const refusals = `const refusal = (promise) => promise.then(() => 'resolved', (e) => [errorCategory(e), e.message.includes('Neo runs attached to BrowserOS')])
let executablePath
try {
  executablePath = playwright.chromium.executablePath()
} catch (e) {
  executablePath = errorCategory(e)
}
return {
  launch: await refusal(playwright.chromium.launch()),
  launchPersistentContext: await refusal(playwright.chromium.launchPersistentContext('profile')),
  launchServer: await refusal(playwright.chromium.launchServer()),
  connect: await refusal(playwright.chromium.connect('ws://127.0.0.1:9')),
  connectOverCDP: await refusal(playwright.chromium.connectOverCDP('http://127.0.0.1:9')),
  executablePath,
  contexts: context.browser().contexts().length,
}`

test('browserType: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages: {} }, async ({ playwright, neo, cdp }) => {
    const reference = outcomeValue(await playwright('/types', source))
    assert.deepEqual(reference, {
      names: ['chromium', 'firefox', 'webkit'],
      own: true,
      firefoxOverCDP: 'Error',
    })
    assert.deepEqual(outcomeValue(await neo('/types', source)), reference)

    // Neo only: every launch and connect is refused, and no browser or context appears.
    const before = await cdp.send('Target.getTargets')
    const refused = await neo('/types', refusals)
    const refusal = ['Error', true]
    assert.deepEqual(outcomeValue(refused), {
      launch: refusal,
      launchPersistentContext: refusal,
      launchServer: refusal,
      connect: refusal,
      connectOverCDP: refusal,
      executablePath: 'Error',
      contexts: 1,
    })
    const after = await cdp.send('Target.getTargets')
    assert.equal(
      after.targetInfos.filter((target) => target.type === 'page').length,
      before.targetInfos.filter((target) => target.type === 'page').length + 1,
      'only the run tab the test opened was added',
    )
  })
})
