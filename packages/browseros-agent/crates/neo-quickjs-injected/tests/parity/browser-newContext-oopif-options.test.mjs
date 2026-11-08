// Regression: server/chromium/crPage.ts FrameSession._initialize applies context
// options before a frame runs. Scripts and CSP must match in cross-site OOPIFs.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { eventually, skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

for (const { name, options, csp, expected } of [
  {
    name: 'javaScriptEnabled:false',
    options: { javaScriptEnabled: false },
    csp: '',
    expected: 'not run',
  },
  {
    name: 'bypassCSP:true',
    options: { bypassCSP: true },
    csp: `<meta http-equiv="Content-Security-Policy" content="script-src 'none'">`,
    expected: 'ran',
  },
]) {
  test(`newContext ${name} applies before an OOPIF runs`, {
    skip,
  }, async (t) => {
    const pages = {
      '/child.html': `${csp}<p id="ran">not run</p><script>document.querySelector('#ran').textContent='ran'</script>`,
    }
    await withPlatform(
      t,
      { pages },
      async ({ playwright, neo, origin, cdp }) => {
        const child = `${origin.replace('127.0.0.1', 'localhost')}/child.html`
        const parent = `${origin}/outer.html`
        pages['/outer.html'] =
          `<iframe src="${child}"></iframe><button id="continue" hidden>continue</button>`
        const source = `const created = await context.browser().newContext(${JSON.stringify(options)})
try {
  const other = await created.newPage()
  await other.goto(${JSON.stringify(parent)})
  const ran = await other.frameLocator('iframe').locator('#ran').textContent()
  await other.locator('#continue').click()
  return ran
} finally { await created.close() }`

        const run = async (engine) => {
          const result = engine('/caller', source)
          let finished = false
          result.then(
            () => {
              finished = true
            },
            () => {
              finished = true
            },
          )
          let target
          let iframeSeen = false
          // Hold the script on a hidden fixture button while the test confirms a real OOPIF.
          await eventually(async () => {
            const { targetInfos } = await cdp.send('Target.getTargets')
            iframeSeen ||= targetInfos.some(
              (info) => info.type === 'iframe' && info.url === child,
            )
            target = targetInfos.find(
              (info) => info.type === 'page' && info.url === parent,
            )
            return finished || (target && iframeSeen)
          }, 'the created page has a cross-site iframe target')
          assert.ok(
            !finished && target && iframeSeen,
            'the script must reach a real OOPIF',
          )
          const { sessionId } = await cdp.send('Target.attachToTarget', {
            targetId: target.targetId,
            flatten: true,
          })
          try {
            await cdp.send(
              'Runtime.evaluate',
              {
                expression: `document.querySelector('#continue').hidden=false`,
              },
              sessionId,
            )
          } finally {
            await cdp
              .send('Target.detachFromTarget', { sessionId })
              .catch(() => {})
          }
          return outcomeValue(await result)
        }
        const reference = await run(playwright)
        const actual = await run(neo)
        t.diagnostic(JSON.stringify({ reference, actual, oopif: true }))
        assert.equal(reference, expected)
        assert.equal(actual, reference)
      },
    )
  })
}
