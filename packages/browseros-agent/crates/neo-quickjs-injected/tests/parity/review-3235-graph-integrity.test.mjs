import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('review: large dispatch graphs reach the main-world constructor intact', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const result = await compareMethod(t, {
    html: `<button id="target">Target</button><script>
      (() => {
        const NativeEvent = window.Event;
        const graphs = [];
        let events = 0;
        window.Event = class Event extends NativeEvent {
          constructor(type, init) {
            if (type === 'ping') {
              const values = init.detail;
              const array = Array.isArray(values);
              graphs.push({
                length: array ? values.length : null,
                dense: array && Object.keys(values).length === values.length,
                shared: array && values.every(value => value === values[0]),
                leaf: array && values[0]?.leaf === true,
              });
            }
            super(type, init);
          }
        };
        target.addEventListener('ping', () => events++);
        window.graphFacts = () => ({ graphs, events });
      })();
    </script>`,
    source: `const shared = { leaf: true };
      for (const length of [10_000, 60_000, 40_000])
        await page.locator('#target').dispatchEvent('ping', { detail: Array(length).fill(shared) });
      return 'done';`,
    probe: 'window.graphFacts()',
    effects: ({ playwright, neo }) => {
      const expected = {
        graphs: [10_000, 60_000, 40_000].map((length) => ({
          length,
          dense: true,
          shared: true,
          leaf: true,
        })),
        events: 3,
      }
      assert.deepEqual(playwright, expected)
      assert.deepEqual(neo, expected)
    },
  })
  assert.equal(result.value, 'done')
})
