// Neo: public final-state parity for concurrent WebStorage calls, cold and warm.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

for (const kind of ['localStorage', 'sessionStorage'])
  test(`${kind} concurrent calls preserve the reference final state`, {
    skip,
    timeout: 180000,
  }, async (t) => {
    await compareMethod(t, {
      html: '<title>storage order</title>',
      source: `
      const storage = page[${JSON.stringify(kind)}]
      try {
        await Promise.all(Array.from({length:20}, (_,i) => storage.setItem('order',String(i))))
        const last = await storage.getItem('order')
        await Promise.all(Array.from({length:20}, (_,i) => storage.setItem('order',String(i+20))))
        const warm = await storage.getItem('order')
        await Promise.all([storage.setItem('a','first'), storage.removeItem('a'), storage.setItem('a','last')])
        return { last, warm, replaced: await storage.getItem('a') }
      } finally { await page.evaluate(() => { localStorage.clear(); sessionStorage.clear() }) }
    `,
      probe: '({local: localStorage.length, session: sessionStorage.length})',
      values({ playwright, neo }) {
        t.diagnostic(JSON.stringify({ playwright, neo }))
        assert.deepEqual(playwright, {
          last: '19',
          warm: '39',
          replaced: 'last',
        })
        assert.deepEqual(neo, playwright)
      },
    })
  })
