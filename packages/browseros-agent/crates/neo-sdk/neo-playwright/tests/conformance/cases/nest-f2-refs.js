// F2 (design D22): each of the 16 refs (a button and a field in each of the 8 frames) read
// back through page.locator('aria-ref=<ref>').ariaSnapshot(), right after its snapshot and
// again after a later one. Each read follows a fresh page snapshot, because upstream forgets a
// frame's refs when a locator in it takes its own snapshot (snapshot-ref-lifetime).
await page.goto(fixtures.url('nest/top.html'))
const snap = async () => {
  let snapshot = ''
  for (let attempt = 0; attempt < 20 && !snapshot.includes('C4 button'); attempt++)
    snapshot = await page.ariaSnapshot({ mode: 'ai' })
  return snapshot
}
const controls = (snapshot) =>
  [...snapshot.matchAll(/- (button|textbox) "([^"]+)" \[ref=([^\]]+)\]/g)].map(
    ([, role, name, ref]) => ({ role, name, ref }),
  )
const resolve = async (found) => {
  const read = {}
  for (const { name, ref } of found) {
    await snap()
    read[name] = await harness.settle(
      page.locator(`aria-ref=${ref}`).ariaSnapshot({ timeout: 2000 }),
    )
  }
  return read
}
const first = controls(await snap())
const now = await resolve(first)
const later = controls(await snap())
return {
  controls: first.length,
  now,
  afterLaterSnapshot: await resolve(first),
  sameRefsLater: JSON.stringify(later) === JSON.stringify(first),
}
