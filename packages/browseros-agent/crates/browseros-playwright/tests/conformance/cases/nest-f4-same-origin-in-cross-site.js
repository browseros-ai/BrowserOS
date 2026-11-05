// F4 (design D22): a same-origin frame inside a cross-site frame inside a same-origin frame
// (top A, a1 A, b2 B out of process, b3 B in b2's process). The a1 subtree as printed, and
// b3's refs read back, each after a fresh page snapshot (see nest-f2-refs).
await page.goto(fixtures.url('nest/top.html'))
const snap = async () => {
  let snapshot = ''
  for (let attempt = 0; attempt < 20 && !snapshot.includes('C4 button'); attempt++)
    snapshot = await page.ariaSnapshot({ mode: 'ai' })
  return snapshot
}
const snapshot = await snap()
const lines = snapshot.split('\n')
const start = lines.findIndex((line) => line.includes('"A1 button"'))
const end = lines.findIndex((line) => line.includes('"B1 button"'))
const b3 = {}
for (const [, name, ref] of snapshot.matchAll(
  /- (?:button|textbox) "(B3 [^"]+)" \[ref=([^\]]+)\]/g,
)) {
  await snap()
  b3[name] = await harness.settle(
    page.locator(`aria-ref=${ref}`).ariaSnapshot({ timeout: 2000 }),
  )
}
return { a1Subtree: lines.slice(start - 1, end).join('\n'), b3 }
