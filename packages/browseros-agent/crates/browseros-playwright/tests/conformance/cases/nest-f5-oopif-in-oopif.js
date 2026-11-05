// F5 (design D22): an out-of-process frame inside an out-of-process frame, four levels deep
// (b1 B, c2 C, a3 A in its own process, c4 C). The b1 subtree as printed, and the refs of
// all four levels read back, each after a fresh page snapshot (see nest-f2-refs).
await page.goto(fixtures.url('nest/top.html'))
const snap = async () => {
  let snapshot = ''
  for (let attempt = 0; attempt < 20 && !snapshot.includes('C4 button'); attempt++)
    snapshot = await page.ariaSnapshot({ mode: 'ai' })
  return snapshot
}
const snapshot = await snap()
const lines = snapshot.split('\n')
const start = lines.findIndex((line) => line.includes('"B1 button"'))
const read = {}
for (const [, name, ref] of snapshot.matchAll(
  /- (?:button|textbox) "((?:B1|C2|A3|C4) [^"]+)" \[ref=([^\]]+)\]/g,
)) {
  await snap()
  read[name] = await harness.settle(
    page.locator(`aria-ref=${ref}`).ariaSnapshot({ timeout: 2000 }),
  )
}
return { b1Subtree: lines.slice(start - 1).join('\n'), read }
