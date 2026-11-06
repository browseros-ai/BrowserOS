// pages.test.mjs opened /other.html, then this run's /run.html (tests/pages/tab.html), and
// drives the browser in step with this script: once both buttons below count a click, it opens
// /opened.html; once the run tab counts its second click, sent only after the click on
// /opened.html resolved, it closes /opened.html.
const path = (p) => p.url().replace(/^https?:\/\/[^/]+/, '');
const until = async (check) => {
  const deadline = Date.now() + 15000;
  while (!check()) {
    if (Date.now() > deadline) throw new Error('context.pages() never changed');
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
};

const before = context.pages();
const again = context.pages();
const other = before.find((p) => path(p) === '/other.html');
// Each click goes through its own tab's Page, both at once.
await Promise.all([page.click('#count'), other.click('#count')]);

let opened;
await until(() => (opened = context.pages().find((p) => path(p) === '/opened.html')));
const withOpened = context.pages();
await opened.click('#count');
await page.click('#count');
await until(() => !context.pages().includes(opened));
const after = context.pages();

return {
  paths: before.map(path),
  isArray: Array.isArray(before),
  runListed: before.includes(page),
  sameObjects: again.length === before.length && again.every((p, i) => p === before[i]),
  openedAppended: withOpened.length === before.length + 1 && withOpened.at(-1) === opened,
  othersKept: before.every((p) => withOpened.includes(p) && after.includes(p)),
  openedLeft: after.length === before.length,
  openedIsClosed: opened.isClosed(),
};
