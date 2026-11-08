import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const fixture = readFileSync(
  new URL('../pages/drag.html', import.meta.url),
  'utf8',
)

test('a click during an HTML5 drag resolves after dropping it', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html: fixture,
    source: `
      await page.mouse.move(60, 40)
      await page.mouse.down()
      await page.mouse.move(250, 60, { steps: 3 })
      let result
      try { await page.mouse.click(250, 60); result = 'resolved' }
      catch (error) { result = errorCategory(error) }
      await page.mouse.up()
      return result
    `,
    probe: 'window.__facts()',
    values: ({ playwright, neo }) => {
      t.diagnostic(JSON.stringify({ playwright, neo }))
      assert.equal(playwright, 'resolved')
      assert.equal(neo, playwright, 'a valid click must resolve')
    },
  })
})

for (const crossSite of [false, true]) {
  test(`dragAndDrop inside a ${crossSite ? 'cross-site' : 'same-origin'} child frame reaches only its target`, {
    skip,
    timeout: 120_000,
  }, async (t) => {
    const html =
      fixture +
      `<script>
      document.querySelector('#left').addEventListener('drop', event => {
        event.currentTarget.dataset.payload = event.dataTransfer.getData('text/plain')
      })
      if (!location.search) {
        const child = document.createElement('iframe')
        child.name = 'child'
        child.id = 'child'
        child.style.cssText = 'position:absolute;left:0;top:280px;width:500px;height:260px;border:0'
        child.src = location.origin${crossSite ? ".replace('127.0.0.1', 'localhost')" : ''} + '/method.html?child=1'
        document.body.appendChild(child)
      }
    </script>`
    await compareMethod(t, {
      html,
      source: `
        await page.locator('#child').waitFor()
        const child = page.frame({ name: 'child' })
        await child.locator('#source').waitFor()
        await child.dragAndDrop('#source', '#left', { steps: 3 })
        return await child.locator('#left').getAttribute('data-payload')
      `,
      probe: 'window.__facts()',
      values: ({ playwright, neo }) => {
        t.diagnostic(JSON.stringify({ playwright, neo }))
        assert.equal(playwright, 'payload')
        assert.equal(neo, playwright, 'the child must receive its payload')
      },
      effects: ({ playwright, neo }) => {
        assert.deepEqual(playwright.events, [], 'parent is a decoy')
        assert.deepEqual(neo, playwright, 'parent remains untouched')
      },
    })
  })
}

test('a drag preserves custom data and the requested copy effect', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html:
      fixture +
      `<script>
      window.__extra = {}
      document.querySelector('#source').addEventListener('dragstart', event => {
        event.dataTransfer.effectAllowed = 'copy'
        event.dataTransfer.setData('application/x-review', 'drag-data-λ')
      })
      document.querySelector('#left').addEventListener('dragover', event => {
        event.dataTransfer.dropEffect = 'copy'
      })
      document.querySelector('#left').addEventListener('drop', event => {
        window.__extra.data = event.dataTransfer.getData('application/x-review')
      })
      document.querySelector('#source').addEventListener('dragend', event => {
        window.__extra.effect = event.dataTransfer.dropEffect
      })
    </script>`,
    source: `await page.locator('#source').dragTo(page.locator('#left'), { steps: 4 }); return 'resolved'`,
    probe: '({ facts: window.__facts(), extra: window.__extra })',
    effects: ({ playwright, neo }) => {
      assert.deepEqual(playwright.extra, {
        data: 'drag-data-λ',
        effect: 'copy',
      })
      assert.deepEqual(neo, playwright)
    },
  })
})

test('force bypasses a covered source and honors source and target positions', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareMethod(t, {
    html:
      fixture +
      `<div id="cover" style="position:absolute;left:20px;top:20px;width:80px;height:40px;background:black"></div>
      <script>
      window.__forced = { down: 0, up: 0 }
      document.querySelector('#cover').addEventListener('mousedown', () => window.__forced.down++)
      document.querySelector('#left').addEventListener('mouseup', () => window.__forced.up++)
      </script>`,
    source: `
      const result = []
      try { await page.dragAndDrop('#source', '#target', { timeout: 400 }); result.push('resolved') }
      catch (error) { result.push(errorCategory(error)) }
      await page.dragAndDrop('#source', '#target', {
        force: true, sourcePosition: { x: 2, y: 2 }, targetPosition: { x: 20, y: 20 }, steps: 3,
      })
      result.push('resolved')
      return result
    `,
    probe: 'window.__forced',
    values: ({ playwright, neo }) => {
      assert.deepEqual(playwright, ['TimeoutError', 'resolved'])
      assert.deepEqual(neo, playwright)
    },
    effects: ({ playwright, neo }) => {
      assert.deepEqual(playwright, { down: 1, up: 1 })
      assert.deepEqual(neo, playwright)
    },
  })
})
