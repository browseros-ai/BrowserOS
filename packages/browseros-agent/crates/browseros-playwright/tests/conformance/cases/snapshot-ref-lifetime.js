// How long an ai snapshot's refs keep resolving on reads.html: after a locator's own
// snapshot, after a default-mode page snapshot, and after a later ai snapshot. Each read has a
// 1 s timeout.
await page.goto(fixtures.url('reads.html'))
const snapshot = await page.ariaSnapshot({ mode: 'ai' })
const ref = (name) => new RegExp(`button "${name}" \\[ref=([^\\]]+)\\]`).exec(snapshot)[1]
const read = (name) =>
  harness.settle(
    page.locator(`aria-ref=${ref(name)}`).textContent({ timeout: 1000 }),
  )
const lifetime = { first: await read('Save') }
await page.locator(`aria-ref=${ref('Save')}`).ariaSnapshot()
lifetime.afterLocatorSnapshot = await read('Cancel order')
await page.ariaSnapshot({ mode: 'ai' })
lifetime.afterNewAiSnapshot = await read('Cancel order')
await page.ariaSnapshot()
lifetime.afterDefaultSnapshot = await read('Save')
return lifetime
