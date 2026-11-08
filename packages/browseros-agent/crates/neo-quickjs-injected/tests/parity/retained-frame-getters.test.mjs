// Neo: Compare retained Frame metadata and actual detach with pinned Playwright after page.close.
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  connectCdp,
  launchReference,
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
    'main document commit then close without a frame getter',
    `await page.goto(page.url().slice(0, page.url().lastIndexOf('/')) + '/new-root');
    await page.close();
    return {
      detached: retained.slice(1).every(frame => frame.isDetached()),
      kept: same.name() === 'same' && same.url().endsWith('/same') && nested.name() === 'nested' && nested.url().endsWith('/nested'),
      parents: same.parentFrame() === main && nested.parentFrame() === same,
      children: retained.every(frame => frame.childFrames().length === 0),
      main: !main.isDetached() && main.url().endsWith('/new-root'),
    };`,
  ],
  [
    'same document navigation then close preserves descendants',
    `await page.goto(page.url() + '#same-doc'); await page.close();
    return {
      attached: retained.every(frame => !frame.isDetached()),
      parents: same.parentFrame() === main && nested.parentFrame() === same,
      children: main.childFrames().length === 2 && same.childFrames()[0] === nested,
      main: main.url().endsWith('#same-doc'),
    };`,
  ],
  [
    'child document commit then close preserves the child',
    `await page.click('#navigate-same'); await page.getByText('same-ready', {exact:true}).waitFor(); await page.close();
    return {
      detached: nested.isDetached(),
      kept: nested.name() === 'nested' && nested.url().endsWith('/nested'),
      parent: nested.parentFrame() === same,
      children: same.childFrames().length === 0 && main.childFrames().includes(same),
      other: !remote.isDetached() && remote.childFrames()[0] === remoteNested,
      same: !same.isDetached() && same.name() === 'same' && same.url().endsWith('/same-next'),
    };`,
  ],
  [
    'same to OOPIF swap retires the old descendant before close',
    `await page.click('#swap-same'); await page.getByText('same-ready', {exact:true}).waitFor(); await page.close();
    return {
      attached: !same.isDetached(), detached: nested.isDetached(),
      kept: nested.name() === 'nested' && nested.url().endsWith('/nested'),
      parents: same.parentFrame() === main && nested.parentFrame() === same,
      children: same.childFrames().length === 0 && main.childFrames().includes(same),
      other: !remote.isDetached() && remote.childFrames()[0] === remoteNested,
    };`,
  ],
  [
    'OOPIF to same swap retires the old descendant before close',
    `await page.click('#swap-remote'); await page.getByText('remote-ready', {exact:true}).waitFor(); await page.close();
    return {
      attached: !remote.isDetached(), detached: remoteNested.isDetached(),
      kept: remoteNested.name() === 'remote-nested' && remoteNested.url().endsWith('/remote-nested'),
      parents: remote.parentFrame() === main && remoteNested.parentFrame() === remote,
      children: remote.childFrames().length === 0 && main.childFrames().includes(remote),
      other: !same.isDetached() && same.childFrames()[0] === nested,
    };`,
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
        '/top': `<iframe name="same" id="same" src="/same"></iframe><iframe name="remote" id="remote"></iframe>
        <button id="remove" onclick="document.querySelector('#remote').remove()">Remove</button>
        <button id="navigate-same" onclick="document.querySelector('#same').src='/same-next'">Navigate same</button>
        <button id="swap-same" onclick="document.querySelector('#same').src=location.origin.replace('127.0.0.1','localhost')+'/same-next'">Swap same</button>
        <button id="swap-remote" onclick="document.querySelector('#remote').src='/remote-next'">Swap remote</button>
        <p id="nav-status">waiting</p>
        <script>
        addEventListener('message', event => { document.querySelector('#nav-status').textContent = event.data; });
        document.querySelector('#remote').src=location.origin.replace('127.0.0.1','localhost')+'/remote';
        </script>`,
        '/same': `<iframe name="nested" src="/nested"></iframe>`,
        '/remote': `<iframe name="remote-nested" src="/remote-nested"></iframe>`,
        '/same-next': `<p>New same document</p><script>parent.postMessage('same-ready','*')</script>`,
        '/remote-next': `<p>New remote document</p><script>parent.postMessage('remote-ready','*')</script>`,
      },
    })
    const { playwright, ...browser } = await launchReference(
      cleanups,
      browserBinary,
    )
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
