// page.pause() through Neo and through Playwright on one BrowserOS neo: with a headless browser
// and no inspector, Playwright's debugger never holds the pause, so it resolves at once, and the
// page's calls go on as before.
//
// Playwright: `client/page.ts` `Page.pause` (pin 26137face), headless and without PWDEBUG; its
// debugger holds the call only when enabled (`tests/library/debugger.spec.ts` "DB-002 should
// pause at pause call" enables it first, which Neo does not serve).
//
// Deviation (`neo-sdk/src/page.ts` `Page.pause`): with a headed browser Playwright opens its
// Inspector and waits for a human; a Neo run is unattended and never waits.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const pages = { '/pause.html': '<title>pause</title><p>paused?</p>' }

const source = `const started = Date.now()
const returned = await page.pause()
return { returned: returned === undefined, quick: Date.now() - started < 5000, title: await page.title(),
  text: await page.locator('p').textContent() }`

test('page.pause: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    const reference = outcomeValue(await playwright('/pause.html', source))
    assert.deepEqual(reference, {
      returned: true,
      quick: true,
      title: 'pause',
      text: 'paused?',
    })
    assert.deepEqual(outcomeValue(await neo('/pause.html', source)), reference)
  })
})
