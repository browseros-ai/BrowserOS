/**
 * BrowserOS's own page UI, kept out of what scripts see. BrowserOS mounts it into every page as
 * an element with an open shadow root, which Playwright's engine pierces like the page's own: on
 * a real BrowserOS page `locator('body')` matched the bar's shadow `<body>` too, a strict
 * violation. So the engine is told that a BrowserOS UI host has no shadow root (the getter in
 * shadow-roots.ts asks `isBrowserOSUi` first), and selectors and counts, text and role matching,
 * accessible names and the aria snapshot all read the page as a browser without the bar shows
 * it. The page and its scripts still see and use the bar. This is the one place the hosts are
 * listed.
 */

/**
 * The hosts BrowserOS mounts, by tag. Seen on a live BrowserOS page: the claw app's takeover bar
 * (`apps/claw-app/entrypoints/takeover.content.ts`) is a `<browseros-neo-takeover>` that WXT's
 * `createShadowRootUi` names after its `name` and appends to `<body>`, with an open shadow root
 * holding its own `<html>`, `<head>` and `<body>`. A new BrowserOS UI root is added here.
 */
const BROWSEROS_UI_HOSTS: ReadonlySet<string> = new Set([
  'browseros-neo-takeover',
])

/** Whether `element` hosts BrowserOS's own UI rather than the page's. */
export function isBrowserOSUi(element: Element): boolean {
  return BROWSEROS_UI_HOSTS.has(element.localName)
}
