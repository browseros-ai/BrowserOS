// The harness opened `page` on <origin>/a. While this script runs, it changes the URL:
// #hash, pushState, a slow navigation, then it closes the tab.
const seen = [page.url()];
while (!page.isClosed()) {
  await new Promise((resolve) => setTimeout(resolve, 50));
  const url = page.url();
  if (url !== seen[seen.length - 1]) seen.push(url);
}
const paths = seen.map((url) => url.slice(url.indexOf('/', 'http://'.length)));
console.log(`saw ${paths.length} URLs before the tab closed`);
return { paths, closed: page.isClosed(), lastPath: paths[paths.length - 1] };
