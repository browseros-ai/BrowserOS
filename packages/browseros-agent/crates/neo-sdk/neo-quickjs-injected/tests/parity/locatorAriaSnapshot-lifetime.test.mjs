import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const child = (label) =>
  `<button onclick="parent.document.querySelector('#effect').textContent='${label} clicked'">${label}</button>`
const install = (label) => `
  const host = document.createElement('div'); host.id = 'host'; document.body.append(host);
  const frame = document.createElement('iframe'); frame.name = 'closed';
  frame.srcdoc = ${JSON.stringify(child(label))};
  host.attachShadow({mode:'closed'}).append(frame);
`
const seeds = `<section id="seed">${Array.from({ length: 20 }, (_, i) => `<button>Seed ${i}</button>`).join('')}</section>`
const before = `${seeds}<output id="effect">untouched</output><script>${install('Old inner')}</script>`
const after = `<output id="effect">untouched</output><script>${install('New inner')}</script>`
const refHelper = `const refOf = text => text.match(/\\[ref=([^\\]]+)\\]/)?.[1]; const stale = locator => locator.count().catch(error => { if (error.name !== 'Error') throw error; return 'Error'; });`

test('closed-shadow owner refs cannot route into a different main document', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: before,
    source:
      refHelper +
      `
      await page.locator('#seed').ariaSnapshot({mode:'ai'});
      const oldButton = page.frame('closed').locator('button');
      await oldButton.waitFor();
      const oldRef = refOf(await oldButton.ariaSnapshot({mode:'ai'}));
      const oldRoute = page.locator('aria-ref=' + oldRef);
      const before = await oldRoute.textContent();
      await page.goto(${JSON.stringify('data:text/html,' + encodeURIComponent(after))});
      const afterNavigation = await stale(oldRoute);
      const freshButton = page.frame('closed').locator('button');
      await freshButton.waitFor();
      const freshRef = refOf(await freshButton.ariaSnapshot({mode:'ai'}));
      const freshRoute = page.locator('aria-ref=' + freshRef);
      await freshRoute.click();
      return {before, afterNavigation, afterFreshSnapshot: await stale(oldRoute),
        fresh: await freshRoute.textContent(), distinct: oldRef !== freshRef};
    `,
    values: ({ playwright, neo }) => {
      assert.deepEqual(playwright, {
        before: 'Old inner',
        afterNavigation: 'Error',
        afterFreshSnapshot: 'Error',
        fresh: 'New inner',
        distinct: true,
      })
      assert.deepEqual(neo, {
        before: 'Old inner',
        afterNavigation: 0,
        afterFreshSnapshot: 0,
        fresh: 'New inner',
        distinct: true,
      })
    },
    probe: `document.querySelector('#effect').textContent`,
  })
  assert.deepEqual(outcome.value, {
    before: 'Old inner',
    afterNavigation: 0,
    afterFreshSnapshot: 0,
    fresh: 'New inner',
    distinct: true,
  })
})

test('a retained disconnected closed-shadow owner never routes to its replacement', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const html =
    before +
    `<button id="replace">Replace</button><script>
    document.querySelector('#replace').onclick = () => {
      window.retainedOldHost = document.querySelector('#host');
      window.retainedOldHost.remove();
      ${install('New inner')}
    };
  </script>`
  const outcome = await compareMethod(t, {
    html,
    source:
      refHelper +
      `
      const oldButton = page.frame('closed').locator('button');
      await oldButton.waitFor();
      const oldRef = refOf(await oldButton.ariaSnapshot({mode:'ai'}));
      const oldRoute = page.locator('aria-ref=' + oldRef);
      const before = await oldRoute.textContent();
      await page.locator('#replace').click();
      const afterReplacement = await stale(oldRoute);
      const freshButton = page.frame('closed').locator('button');
      await freshButton.waitFor();
      const freshRef = refOf(await freshButton.ariaSnapshot({mode:'ai'}));
      const freshRoute = page.locator('aria-ref=' + freshRef);
      await freshRoute.click();
      return {before, afterReplacement, oldCount: await stale(oldRoute), fresh: await freshRoute.textContent()};
    `,
    values: ({ playwright, neo }) => {
      assert.deepEqual(playwright, {
        before: 'Old inner',
        afterReplacement: 'Error',
        oldCount: 'Error',
        fresh: 'New inner',
      })
      assert.deepEqual(neo, {
        before: 'Old inner',
        afterReplacement: 0,
        oldCount: 0,
        fresh: 'New inner',
      })
    },
    probe: `({effect: document.querySelector('#effect').textContent, oldConnected: window.retainedOldHost.isConnected})`,
  })
  assert.deepEqual(outcome.value, {
    before: 'Old inner',
    afterReplacement: 0,
    oldCount: 0,
    fresh: 'New inner',
  })
})

test('an owner adopted by another document does not retain its former root path', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const html = `<output id="effect">untouched</output><div id="source-host"></div>
    <iframe id="destination" name="destination" srcdoc="<p>Destination</p>"></iframe>
    <button id="move">Move owner</button>
    <script>
      const owner = document.createElement('iframe'); owner.name = 'moved';
      owner.srcdoc = '<button>Moved inner</button>';
      document.querySelector('#source-host').attachShadow({mode:'closed'}).append(owner);
      document.querySelector('#move').onclick = () =>
        document.querySelector('#destination').contentDocument.body.append(document.querySelector('#source-host'));
    </script>`
  const outcome = await compareMethod(t, {
    html,
    source:
      refHelper +
      `
      const original = page.frame('moved').locator('button');
      await original.waitFor();
      const oldRef = refOf(await original.ariaSnapshot({mode:'ai'}));
      const oldRoute = page.locator('aria-ref=' + oldRef);
      const before = await oldRoute.textContent();
      await page.locator('#move').click();
      await page.frameLocator('#destination').locator('#source-host').waitFor({state:'attached'});
      const afterMove = await stale(oldRoute);
      const moved = page.frame('moved');
      const freshButton = moved.locator('button');
      await freshButton.waitFor();
      const freshRef = refOf(await freshButton.ariaSnapshot({mode:'ai'}));
      return {before, afterMove, afterFreshSnapshot: await stale(oldRoute),
        fresh: await page.locator('aria-ref=' + freshRef).textContent(), parent: moved.parentFrame().name(), refs:{old:oldRef,fresh:freshRef}};
    `,
    values: ({ playwright, neo }) => {
      t.diagnostic(
        JSON.stringify({ playwrightRefs: playwright.refs, neoRefs: neo.refs }),
      )
      assert.notEqual(playwright.refs.old, playwright.refs.fresh)
      assert.notEqual(neo.refs.old, neo.refs.fresh)
      const { refs: pwRefs, ...pwValue } = playwright
      const { refs: neoRefs, ...neoValue } = neo
      assert.deepEqual(pwValue, {
        before: 'Moved inner',
        afterMove: 'Error',
        afterFreshSnapshot: 'Error',
        fresh: 'Moved inner',
        parent: 'destination',
      })
      assert.deepEqual(neoValue, {
        before: 'Moved inner',
        afterMove: 0,
        afterFreshSnapshot: 0,
        fresh: 'Moved inner',
        parent: 'destination',
      })
    },
    probe: `document.querySelector('#destination').contentDocument.querySelector('#source-host') !== null`,
  })
  const { refs, ...value } = outcome.value
  assert.deepEqual(value, {
    before: 'Moved inner',
    afterMove: 0,
    afterFreshSnapshot: 0,
    fresh: 'Moved inner',
    parent: 'destination',
  })
})
