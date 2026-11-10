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
const opened = {
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

// Each call must reach the tab of the page it goes through.
const clicked = await second.locator('html').click();
const fronted = await second.bringToFront();
const closed = await first.close();
// Opened once `second` is in front: Playwright opens it in front, Neo behind `second`.
const third = await context.newPage();
const end = context.pages();

return {
  ...opened,
  voidResults: [clicked, fronted, closed].map((value) => value === undefined),
  firstIsClosed: first.isClosed(),
  listedAtEnd: [first, second, third].map((p) => end.includes(p)),
};
