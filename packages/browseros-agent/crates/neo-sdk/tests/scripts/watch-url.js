// The harness opened `page` on <origin>/a. While this script runs, it changes the URL:
// #hash, pushState, a navigation to /slow, whose response the fixture holds back for 2.5 s,
// then it closes the tab.
const path = (url) => url.slice(url.indexOf('/', 'http://'.length));
const seen = [{ path: path(page.url()), at: Date.now() }];
while (!page.isClosed()) {
  await new Promise((resolve) => setTimeout(resolve, 50));
  const current = path(page.url());
  if (current !== seen[seen.length - 1].path) seen.push({ path: current, at: Date.now() });
}
const firstSeen = (wanted) => seen.find((entry) => entry.path === wanted)?.at;
console.log(`saw ${seen.length} URLs before the tab closed`);
return {
  paths: seen.map((entry) => entry.path),
  // The URL changes when a navigation commits, so `/pushed` stays through the whole load of
  // `/slow`. A URL that jumped to the destination when loading started would replace it
  // about 300 ms after `/pushed` appeared.
  keptOldUrlWhileLoading: firstSeen('/slow') - firstSeen('/pushed') > 1500,
  // Read after the close on purpose: the last committed URL, not an error.
  afterClose: path(page.url()),
};
