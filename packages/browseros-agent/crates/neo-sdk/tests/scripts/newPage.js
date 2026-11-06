// newPage.test.mjs opened this run's /run.html (tests/pages/tab.html). After the script it reads
// which tabs exist and which one is active.
const before = context.pages();
const first = await context.newPage();
const second = await context.newPage();
const after = context.pages();
let crossFrameThrows = false;
try {
  first.locator('#count').or(second.locator('#count'));
} catch {
  crossFrameThrows = true;
}

return {
  urls: [first.url(), second.url()],
  isClosed: [first.isClosed(), second.isClosed()],
  distinct: first !== second && first !== page && second !== page,
  context: first.context() === context && second.context() === context,
  ownMainFrames:
    first.mainFrame() !== second.mainFrame() &&
    first.mainFrame().page() === first &&
    second.mainFrame().page() === second,
  crossFrameThrows,
  appendedInOrder:
    after.length === before.length + 2 && after.at(-2) === first && after.at(-1) === second,
  othersKept: before.every((p) => after.includes(p)),
};
