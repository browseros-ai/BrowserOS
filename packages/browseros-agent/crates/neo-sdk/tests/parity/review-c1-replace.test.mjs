import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('adding a handler while removing the last old handler keeps event delivery', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html: '<div id="old" hidden>Old</div><div id="banner">New</div><button id="target" onclick="window.clicked++">Target</button><script>window.clicked=0;window.handled=0</script>',
    source: `
      const old = page.locator('#old');
      await page.addLocatorHandler(old, () => {});
      await Promise.all([
        page.addLocatorHandler(page.locator('#banner'), async () => {
          await page.evaluate(() => {
            window.handled++;
            document.querySelector('#banner').hidden = true;
          });
        }),
        page.removeLocatorHandler(old),
      ]);
      await page.locator('#target').click();
      return await page.evaluate(() => ({ handled, clicked }));
    `,
    probe:
      '({handled,clicked,bannerVisible:!document.querySelector("#banner").hidden})',
    values: ({ playwright, neo }) => {
      t.diagnostic(JSON.stringify({ playwright, neo }))
      assert.deepEqual(playwright, { handled: 1, clicked: 1 })
      assert.deepEqual(neo, playwright)
    },
  })
})
