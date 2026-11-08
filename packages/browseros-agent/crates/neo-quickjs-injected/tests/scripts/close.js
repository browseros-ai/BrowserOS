// close.test.mjs opened /closed.html, /unloaded.html and /kept.html, then this run's /run.html
// (tests/pages/tab.html). Each kind of close goes to a tab of its own; after the script the test
// reads which tabs are gone and which ran their beforeunload handlers.
const path = (p) => p.url().replace(/^https?:\/\/[^/]+/, '');
const listed = context.pages();
const tab = (name) => listed.find((p) => path(p) === name);
const closed = tab('/closed.html');
const unloaded = tab('/unloaded.html');
const kept = tab('/kept.html');
const failure = (promise) =>
  promise.then(
    () => 'resolved',
    (error) => ({ name: error.name, saysClosed: error.message.includes('closed') }),
  );

const result = {};
result.closeReturnsUndefined = (await closed.close()) === undefined;
result.closedIsClosed = closed.isClosed();
result.closedListed = context.pages().includes(closed);
result.closedUrl = path(closed);
result.closeAgainReturnsUndefined = (await closed.close({ reason: 'done' })) === undefined;
result.runBeforeUnloadWhenClosed = await failure(closed.close({ runBeforeUnload: true }));
result.badReason = await failure(kept.close({ reason: 5 }));
result.keptIsClosed = kept.isClosed();
// Playwright tests runBeforeUnload for truth, so 'yes' asks for it too. That close resolves
// without waiting for the tab to go.
result.runBeforeUnloadReturnsUndefined =
  (await unloaded.close({ runBeforeUnload: 'yes' })) === undefined;
const deadline = Date.now() + 15000;
while (!unloaded.isClosed() && Date.now() < deadline) {
  await new Promise((resolve) => setTimeout(resolve, 50));
}
result.unloadedIsClosed = unloaded.isClosed();
const after = context.pages();
result.listedAfter = [page, kept, closed, unloaded].map((p) => after.includes(p));
return result;
