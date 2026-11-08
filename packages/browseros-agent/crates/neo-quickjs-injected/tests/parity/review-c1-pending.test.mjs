import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('removing a running handler does not release the action waiting for it', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html: '<div id="banner">Banner</div><button id="target" onclick="window.clicked++">Target</button><script>window.clicked=0</script>',
    source: `
      let entered, release, finished;
      const started = new Promise(resolve => { entered = resolve });
      const blocked = new Promise(resolve => { release = resolve });
      const completed = new Promise(resolve => { finished = resolve });
      const banner = page.locator('#banner');
      await page.addLocatorHandler(banner, async () => {
        entered();
        await blocked;
        finished();
      }, { noWaitAfter: true });
      const action = page.locator('#target').click({ timeout: 700 })
        .then(() => 'resolved', errorCategory);
      await started;
      await page.removeLocatorHandler(banner);
      const result = await action;
      release();
      await completed;
      return { result, clicked: await page.evaluate(() => window.clicked) };
    `,
    probe: 'window.clicked',
    values: ({ playwright, neo }) => {
      t.diagnostic(JSON.stringify({ playwright, neo }))
      assert.deepEqual(playwright, { result: 'TimeoutError', clicked: 0 })
      assert.deepEqual(neo, playwright)
    },
  })
})
