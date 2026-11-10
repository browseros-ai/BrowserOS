// page.ariaSnapshot({ mode: 'ai' }) on reads.html, then every ref it printed read back
// through page.locator('aria-ref=<ref>'): each ref names the element printed for it. The reads
// use textContent, which leaves the page's refs alone (snapshot-ref-lifetime covers reads
// that take snapshots of their own).
await page.goto(fixtures.url('reads.html'))
const snapshot = await page.ariaSnapshot({ mode: 'ai' })
const refs = [...snapshot.matchAll(/\[ref=([^\]]+)\]/g)].map((match) => match[1])
const resolved = {}
for (const ref of refs)
  resolved[ref] = await harness.settle(
    page.locator(`aria-ref=${ref}`).textContent({ timeout: 1000 }),
  )
const again = await page.ariaSnapshot({ mode: 'ai' })
return {
  snapshot,
  resolved,
  sameRefsAgain: again === snapshot,
  unknownRef: await harness.settle(
    page.locator('aria-ref=e999').textContent({ timeout: 500 }),
  ),
}
