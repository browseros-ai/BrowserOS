// bringToFront.test.mjs opened /front.html and /gone.html, then this run's /run.html
// (tests/pages/tab.html), the active tab. After the script it reads which tab is active and
// which one counted the click.
const path = (p) => p.url().replace(/^https?:\/\/[^/]+/, '');
const listed = context.pages();
const front = listed.find((p) => path(p) === '/front.html');
const gone = listed.find((p) => path(p) === '/gone.html');

const result = {};
result.returnsUndefined = (await front.bringToFront()) === undefined;
result.againReturnsUndefined = (await front.bringToFront()) === undefined;
await gone.close();
result.whenClosed = await gone.bringToFront().then(
  () => 'resolved',
  (error) => ({ name: error.name, saysClosed: error.message.includes('closed') }),
);
result.frontIsClosed = front.isClosed();
await front.click('#count');
return result;
