// Independent R2 checks: page isolation and evaluation without action prechecks.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareEvents } from './events.mjs'
import { compareMethod, skip } from './harness.mjs'

test('identity refs stay on their page and malformed identities reject', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: `<output id="effect">untouched</output><iframe name="hidden" style="visibility:hidden"></iframe>
      <script>
        if (location.search === '?inner') {
          document.body.innerHTML = '<button onclick="parent.postMessage(1,&#39;*&#39;)">Hidden</button>';
        } else {
          document.querySelector('iframe').src = 'http://localhost:' + location.port + '/method.html?inner';
          addEventListener('message', event => {
            if (event.data === 1) document.querySelector('#effect').textContent = 'clicked';
          });
        }
      </script>`,
    source: `
      const refOf = text => text.match(/\\[ref=([^\\]]+)\\]/)[1];
      const target = page.frame('hidden').locator('button');
      await target.waitFor({state:'attached'});
      const ref = refOf(await target.ariaSnapshot({mode:'ai'}));
      const other = await page.context().newPage();
      await other.goto(page.url());
      const otherTarget = other.frame('hidden').locator('button');
      await otherTarget.waitFor({state:'attached'});
      const otherRef = refOf(await otherTarget.ariaSnapshot({mode:'ai'}));
      const foreign = await other.locator('aria-ref=' + ref).count().then(String, errorCategory);
      const own = await other.locator('aria-ref=' + otherRef).textContent();
      await other.evaluate(() => document.querySelector('iframe').style.visibility = 'visible');
      await other.locator('aria-ref=' + otherRef).click();
      await other.waitForFunction(() => document.querySelector('#effect').textContent === 'clicked');
      const otherEffect = await other.locator('#effect').textContent();
      const bad = [
        'fi' + 'a'.repeat(31) + 'e1',
        'fi' + 'A'.repeat(32) + 'e1',
        'fi' + 'g'.repeat(32) + 'e1',
        'fi' + 'a'.repeat(33) + 'e1',
      ];
      const invalid = [];
      for (const ref of bad)
        invalid.push(await page.locator('aria-ref=' + ref).count().then(String, errorCategory));
      await other.close();
      return {foreign,own,otherEffect,invalid,original:await page.locator('aria-ref=' + ref).textContent()};
    `,
    probe: 'document.querySelector("#effect").textContent',
    values: ({ playwright, neo }) => {
      // Playwright's frame numbers start over on each page; this string names the other
      // page's local frame. Neo's identity token must never name a foreign frame.
      assert.equal(playwright.foreign, '1')
      assert.equal(neo.foreign, 'Error')
      const { foreign: _playwrightForeign, ...expected } = playwright
      const { foreign: _neoForeign, ...actual } = neo
      assert.deepEqual(actual, expected)
    },
  })
  assert.deepEqual(outcome.value, {
    foreign: 'Error',
    own: 'Hidden',
    otherEffect: 'clicked',
    invalid: ['0', '0', '0', '0'],
    original: 'Hidden',
  })
})

test('evaluate rejects when its pending promise loses its document to navigation', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: '<h1>Old</h1>',
    slowMs: 1000,
    source: `
      const result = await page.evaluate(() => {
        location.href = '/slow';
        return new Promise(() => {});
      }).then(() => 'resolved', errorCategory);
      await page.waitForURL(/\\/slow$/);
      return result;
    `,
    probe: 'document.querySelector("h1").textContent',
  })
  assert.equal(outcome.value, 'Error')
})

test('evaluation on blank and closed pages and through dialog dismissal', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/plain.html': '<title>Ready</title>',
    },
    cases: [
      {
        name: 'about:blank evaluation and closed-page rejection keep their categories',
        path: '/plain.html',
        source: `
          const blank = await context.newPage();
          const value = await blank.evaluate(() => { globalThis.answer = 42; return answer });
          await blank.close();
          const closed = await blank.evaluate(() => 1).then(String, errorCategory);
          return {value,closed};
        `,
        expected: { value: 42, closed: 'Error' },
      },
      {
        name: 'evaluation queued during a dialog settles after dismissal',
        path: '/plain.html',
        source: `
          let opened;
          const dialog = new Promise(resolve => { opened = resolve });
          page.once('dialog', opened);
          const opening = page.evaluate(() => alert('hold'));
          const current = await dialog;
          const pending = page.evaluate(() => { document.title = 'After'; return document.title });
          await current.dismiss();
          await opening;
          return await pending;
        `,
        expected: 'After',
      },
    ],
  })
})

test('a first-call script tag navigation is followed by the merged locator snapshot', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: '<h1>Old</h1>',
    slowMs: 1000,
    source: `
      await page.addScriptTag({content: "location.href = '/slow'"});
      await new Promise(resolve => setTimeout(resolve, 300));
      return await page.locator('h1').ariaSnapshot();
    `,
    probe: 'document.querySelector("h1").textContent',
  })
  assert.equal(outcome.value, '- heading "/slow" [level=1]')
})
