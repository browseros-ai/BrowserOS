// F1 (design D22): the ai snapshot of the 4-level, 3-site nested fixture. Every frame's
// content sits under its parent's `- iframe` line, with `e` refs in the main frame and
// `f<seq>e` refs in each child frame.
await page.goto(fixtures.url('nest/top.html'))
let snapshot = ''
for (let attempt = 0; attempt < 20 && !snapshot.includes('C4 button'); attempt++)
  snapshot = await page.ariaSnapshot({ mode: 'ai' })
return {
  snapshot,
  frames: page.frames().length,
}
