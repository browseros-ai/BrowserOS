// Neo: compare scoped snapshots and ref actions with Playwright on fresh fixture pages.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('tests that locator snapshots match Playwright and scoped refs work', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/locator-snapshot.html', import.meta.url),
      'utf8',
    ),
    source: `
      const section = page.getByRole('main').locator('section').filter({ hasText: 'Featured' })
      const plain = await section.ariaSnapshot()
      const shallow = await section.ariaSnapshot({ depth: 2 })
      const ai = await section.ariaSnapshot({ mode: 'ai' })
      const ref = ai.split('\\n').find(line => line.includes('button "Buy apple"'))
        ?.match(/\\[ref=(e\\d+)\\]/)?.[1]
      await page.locator('aria-ref=' + ref).click()
      const refText = await page.locator('aria-ref=' + ref).textContent()
      const strict = []
      for (const mode of ['default', 'ai'])
        strict.push(await page.getByRole('button').ariaSnapshot({ mode }).catch(errorCategory))
      const noMatch = await page.getByRole('region', { name: 'Missing' })
        .ariaSnapshot({ mode: 'ai' }).catch(errorCategory)
      return { plain, shallow, ai, refText, strict, noMatch }
    `,
    probe: () => document.querySelector('#result').textContent,
  })
  assert.doesNotMatch(outcome.value.plain, /Outside|Other products/)
  assert.notEqual(outcome.value.shallow, outcome.value.plain)
  assert.match(outcome.value.ai, /\[ref=e\d+\]/)
  assert.equal(outcome.value.refText, 'Buy apple')
  assert.deepEqual(outcome.value.strict, ['strict', 'strict'])
  assert.equal(outcome.value.noMatch, 'error')
})
