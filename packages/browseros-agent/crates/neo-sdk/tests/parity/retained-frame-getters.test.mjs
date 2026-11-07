// Neo: Compare retained Frame metadata and actual detach with pinned Playwright after page.close.
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  connectCdp,
  launchBrowser,
  openTab,
  playwrightVersion,
  runCleanups,
  runInNeo,
  runInPlaywright,
  serveFixture,
  skip,
} from './harness.mjs'

// Neo: One shared script observes canonical objects before closing the real page.
const setup = `
const until = Date.now() + 10000;
let frames;
do {
  frames = page.frames();
  if (frames.length === 5 && frames.every(frame => frame.url() !== 'about:blank')) break;
  await new Promise(resolve => setTimeout(resolve, 10));
} while (Date.now() < until);
if (frames.length !== 5) throw new Error('Expected all five frames');
const main = page.mainFrame();
const same = page.frame('same');
const nested = page.frame('nested');
const remote = page.frame('remote');
const remoteNested = page.frame('remote-nested');
const retained = [main, same, nested, remote, remoteNested];
`

// Neo: Each method's public return value is compared, including canonical relationships.
const cases = [
  ['names', `await page.close(); return retained.map(frame => frame.name());`],
  ['URLs', `await page.close(); return retained.map(frame => frame.url());`],
  [
    'updated main URL',
    `await page.goto(page.url().slice(0, page.url().lastIndexOf('/')) + '/new-root'); const observed = main.url(); await page.close(); return observed.endsWith('/new-root') && main.url() === observed;`,
  ],
  [
    'detach status',
    `await page.close(); return retained.map(frame => frame.isDetached());`,
  ],
  [
    'parents',
    `await page.close(); return [main.parentFrame() === null, same.parentFrame() === main, nested.parentFrame() === same, remote.parentFrame() === main, remoteNested.parentFrame() === remote];`,
  ],
  [
    'children',
    `await page.close(); return [main.childFrames().length === 2, main.childFrames()[0] === same, main.childFrames()[1] === remote, same.childFrames()[0] === nested, remote.childFrames()[0] === remoteNested, nested.childFrames().length === 0, remoteNested.childFrames().length === 0];`,
  ],
  ...[false, true].map((refresh) => [
    `real detach before close; refresh=${refresh}`,
    `
    await page.click('#remove');
    ${refresh ? "if (!remote.isDetached()) throw new Error('Expected real detach');" : ''}
    await page.close();
    return {
      detached: remote.isDetached() && remoteNested.isDetached(),
      parents: remote.parentFrame() === main && remoteNested.parentFrame() === remote,
      children: remote.childFrames().length === 0 && remoteNested.childFrames().length === 0 && main.childFrames().length === 1 && main.childFrames()[0] === same,
      kept: remote.name() === 'remote' && remote.url().endsWith('/remote') && remoteNested.name() === 'remote-nested' && remoteNested.url().endsWith('/remote-nested'),
      surviving: !same.isDetached() && !nested.isDetached(),
    };
  `,
  ]),
  [
    'closed actions reject',
    `await page.close(); const name = remote.name(); try { await remote.title(); return false; } catch (error) { return name === 'remote' && error.name === 'Error'; }`,
  ],
]

test('retained frame getters match Playwright after page close', {
  skip,
  timeout: 180_000,
}, async (t) => {
  t.diagnostic(
    `Playwright ${playwrightVersion}; ${process.env.NEO_PARITY_PLAYWRIGHT ?? 'npm playwright-core'}`,
  )
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: {
        '/top': `<iframe name="same" src="/same"></iframe><iframe name="remote" id="remote"></iframe><button id="remove" onclick="document.querySelector('#remote').remove()">Remove</button><script>document.querySelector('#remote').src=location.origin.replace('127.0.0.1','localhost')+'/remote'</script>`,
        '/same': `<iframe name="nested" src="/nested"></iframe>`,
        '/remote': `<iframe name="remote-nested" src="/remote-nested"></iframe>`,
      },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const playwright = await chromium.connectOverCDP(
      `http://127.0.0.1:${browser.port}`,
    )
    cleanups.push(() => playwright.close())
    const context = playwright.contexts()[0]
    const cdp = await connectCdp(cleanups, browser.port)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-retained-frames-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    for (const [index, [name, body]] of cases.entries()) {
      await t.test(name, async () => {
        const source = setup + body
        const reference = await openTab(cdp, `${origin}/top`)
        const expected = await runInPlaywright(
          context,
          reference.targetId,
          source,
        )
        assert.ok('value' in expected, JSON.stringify(expected))
        const tab = await openTab(cdp, `${origin}/top`)
        const path = join(scripts, `${index}.js`)
        writeFileSync(path, source)
        const actual = await runInNeo(neoRun, browser.port, tab.targetId, path)
        assert.equal(
          actual.outcome.status,
          'ok',
          JSON.stringify(actual.outcome),
        )
        assert.deepEqual(actual.value, expected.value)
      })
    }
  } finally {
    await runCleanups(cleanups)
  }
})
