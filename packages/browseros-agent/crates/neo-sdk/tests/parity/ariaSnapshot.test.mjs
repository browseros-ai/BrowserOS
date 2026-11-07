// page.ariaSnapshot through the Playwright reference and through Neo, on fresh pages of the dialog
// fixture: the default, ai and depth-limited snapshots as returned strings, an ai ref clicked and
// read through page.locator('aria-ref=…'), the click's effect in the page (the button's confirm()
// was answered, as both dismiss a dialog nobody listens for), and an invalid mode rejected,
// compared by kind only (the error's name), never the wording.
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases, value } from './frames.mjs'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  connectCdp,
  launchBrowser,
  playwrightVersion,
  runCleanups,
  serveFixture,
  skip,
  startNeo,
  waitForUrl,
} from './harness.mjs'

const source = `
  const rejected = (e) => ({
    rejected: true,
    name: e.name,
    strict: /strict mode violation/.test(e.message),
  })
  const plain = await page.ariaSnapshot()
  const ai = await page.ariaSnapshot({ mode: 'ai' })
  const shallow = await page.ariaSnapshot({ mode: 'ai', depth: 1 })
  const ref = ai
    .split('\\n')
    .find((line) => line.includes('button "Submit"'))
    ?.match(/\\[ref=(e\\d+)\\]/)?.[1]
  await page.locator('aria-ref=' + ref).click()
  const refText = await page.locator('aria-ref=' + ref).textContent()
  return {
    plain,
    ai,
    shallow,
    type: typeof ai,
    ref,
    refText,
    invalidMode: await page.ariaSnapshot({ mode: 'bogus' }).catch(rejected),
  }
`

test('ariaSnapshot returns the same YAML and its refs click and read', {
  skip,
  timeout: 180_000,
}, async (t) => {
  t.diagnostic(`Playwright reference: ${playwrightVersion}`)
  const binary = buildNeoRun()
  const cleanups = []
  try {
    const html = readFileSync(
      new URL('../pages/dialog.html', import.meta.url),
      'utf8',
    )
    const origin = await serveFixture(cleanups, { pages: { '/dialog': html } })
    const { port } = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, port)
    const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
    cleanups.push(() => browser.close())
    const page = await browser.contexts()[0].newPage()
    await page.goto(`${origin}/dialog`)
    const reference = await new (async () => {}).constructor('page', source)(
      page,
    )
    const referenceAnswer = await page.evaluate(() => window.__answer)

    const { targetId } = await cdp.send('Target.createTarget', {
      url: `${origin}/dialog`,
    })
    cleanups.push(() => cdp.send('Target.closeTarget', { targetId }))
    const { sessionId } = await cdp.send('Target.attachToTarget', {
      targetId,
      flatten: true,
    })
    await waitForUrl(cdp, sessionId, `${origin}/dialog`)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-aria-snapshot-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const script = join(scripts, 'aria-snapshot.js')
    writeFileSync(script, source)
    const outcome = await startNeo(binary, port, targetId, script).outcome
    const answer = await cdp.send(
      'Runtime.evaluate',
      { expression: 'window.__answer', returnByValue: true },
      sessionId,
    )

    assert.deepEqual(reference, {
      plain: '- button "Submit"',
      ai: '- button "Submit" [ref=e2]',
      shallow: '- button "Submit" [ref=e2]',
      type: 'string',
      ref: 'e2',
      refText: 'Submit',
      invalidMode: { rejected: true, name: 'Error', strict: false },
    })
    assert.equal(
      referenceAnswer,
      false,
      'the dismissed confirm() answered false',
    )
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    assert.deepEqual(outcome.value, reference, 'Neo vs Playwright')
    assert.equal(
      answer.result.value,
      referenceAnswer,
      'the click reached the page',
    )
  } finally {
    await runCleanups(cleanups)
  }
})

/**
 * page.ariaSnapshot over frames, on tabs of frame-top.html (frames.mjs): the `ai` snapshot
 * stitches each visible iframe's content under its `- iframe` node, out-of-process frames and
 * the mixed tree included, also with a depth limit; the default snapshot does not (design §3.6).
 * Refs read from the snapshot act inside frames: an Email inside the cross-site frame is filled
 * and read back, a button in the 127.0.0.1 frame inside it is clicked, and the iframe's own ref
 * leads into its frame through contentFrame(). The YAML is compared with refs blanked.
 *
 * Known, intended difference, asserted per engine (design §4.3): ref strings inside frames.
 * Playwright prefixes a frame's refs with its frame number (`f4e3`); Neo with the iframe's ref
 * number at each level (`- iframe [ref=e23]` → `f23e3`, one level deeper `f23f12e5`), so a later
 * run can rebuild them. The local part (`e3`) names the same element on both.
 */
const frameCases = [
  {
    name: 'the ai snapshot stitches frames and its refs act inside them',
    source: `const refOf = (line) => /\\[ref=([^\\]]+)\\]/.exec(line)?.[1]
    // BrowserOS neo's takeover bar (<browseros-neo-takeover>, which claw-app's content script adds
    // to every page at document_idle) shows as one '- document' line, or not, depending on when
    // the snapshot runs; it is not the page's, so both engines drop it.
    const own = (yaml) => yaml.split('\\n').filter((line) => line.trim() !== '- document').join('\\n')
    const blank = (yaml) => yaml.replace(/\\[ref=[^\\]]+\\]/g, '[ref]')
    const plain = own(await page.ariaSnapshot())
    const shallow = own(await page.ariaSnapshot({ mode: 'ai', depth: 3 }))
    const ai = own(await page.ariaSnapshot({ mode: 'ai' }))
    const lines = ai.split('\\n')
    const after = (marker, role) =>
      refOf(lines.slice(lines.findIndex((line) => line.includes(marker))).find((line) => line.includes(role)))
    const crossEmail = after(': cross gen 1', 'textbox "Email"')
    const grandchildSend = after(': grandchild gen 1', 'button "Send"')
    const iframes = lines.filter((line) => line.trimStart().startsWith('- iframe ')).map(refOf)
    // A snapshot without a frame's content has no ref to act on: report that, not a timeout.
    const act = (ref, call) => (ref === undefined ? 'no ref' : settle(() => call(page.locator('aria-ref=' + ref))))
    return {
      plain,
      shallow: blank(shallow),
      ai: blank(ai),
      filled: await act(crossEmail, (target) => target.fill('via-ref')),
      read: await act(crossEmail, (target) => target.inputValue()),
      clicked: await act(grandchildSend, (target) => target.click()),
      iframeName: await act(iframes[1], (target) => target.getAttribute('name')),
      throughIframe: await act(iframes[1], (target) => target.contentFrame().locator('#who').textContent()),
      refs: { crossEmail, grandchildSend, iframes },
    }`,
    perEngine: ['refs'],
    expected: {
      plain: (plain) => {
        assert.equal(
          plain.match(/^- iframe$/gm)?.length,
          3,
          'iframes as leaves',
        )
        assert.doesNotMatch(plain, /(same|cross|chain|grandchild) gen 1/)
      },
      shallow: (shallow) => {
        assertStitched(shallow)
        assert.ok(
          !shallow.includes('same one'),
          'the depth limit cuts frames too',
        )
      },
      ai: (ai) => {
        assertStitched(ai)
        assert.ok(ai.includes('- listitem [ref]: same one'))
      },
      filled: 'undefined',
      read: value('via-ref'),
      clicked: 'undefined',
      iframeName: value('cross'),
      throughIframe: value('cross'),
    },
    effect: (effect) => {
      const { cross, grandchild } = assertDecoysUntouched(effect, [
        'cross',
        'grandchild',
      ])
      assert.equal(cross.email, 'via-ref')
      assert.deepEqual(
        grandchild.events.filter((event) => event.type === 'click'),
        [{ type: 'click', target: 'send', trusted: true }],
        `the click reached the grandchild's Send: ${JSON.stringify({ grandchild: grandchild.events, cross: cross.events })}`,
      )
    },
    deliveries: { fill: ['done'], click: ['done'] },
    checkPlaywright: ({ result }) => {
      const { crossEmail, grandchildSend, iframes } = result.value.refs
      assert.match(
        crossEmail,
        /^f\d+e\d+$/,
        'Playwright: frame number + local ref',
      )
      assert.match(grandchildSend, /^f\d+e\d+$/)
      assert.equal(iframes.length, 5, 'five rendered iframes')
    },
    checkNeo: ({ result }, playwright) => {
      const { crossEmail, grandchildSend, iframes } = result.value.refs
      const reference = playwright.result.value.refs
      const number = (ref) => /e(\d+)$/.exec(ref)[1]
      const local = (ref) => /e\d+$/.exec(ref)[0]
      const [, cross, nested] = iframes
      assert.match(
        cross,
        /^e\d+$/,
        "the cross-site iframe's own ref is a main-frame ref",
      )
      assert.equal(nested, `f${number(cross)}e${number(reference.iframes[2])}`)
      assert.equal(
        crossEmail,
        `f${number(cross)}${local(reference.crossEmail)}`,
      )
      assert.equal(
        grandchildSend,
        `f${number(cross)}f${number(nested)}${local(reference.grandchildSend)}`,
      )
    },
  },
]

/**
 * Asserts that `yaml` (refs blanked) shows each visible frame's document under its `- iframe`
 * node, nested frames one level deeper, and no hidden iframe.
 */
function assertStitched(yaml) {
  const indent = (level) => '  '.repeat(level)
  for (const [who, level] of [
    ['same', 1],
    ['cross', 1],
    ['grandchild', 3],
    ['chain', 1],
    ['chain-inner', 3],
  ])
    assert.ok(
      yaml.includes(
        `${indent(level)}- iframe [ref]:\n${indent(level + 1)}- generic [ref]:\n${indent(level + 2)}- paragraph [ref]: ${who} gen 1\n`,
      ),
      `${who}'s document under its iframe`,
    )
  assert.doesNotMatch(yaml, /hidden-(same|cross)/, 'hidden iframes stay out')
}

test('ariaSnapshot over frames reads the same tree through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
