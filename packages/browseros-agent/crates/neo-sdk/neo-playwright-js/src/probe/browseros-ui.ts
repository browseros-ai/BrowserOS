/**
 * BrowserOS's own page UI, kept out of what scripts see. BrowserOS mounts it into every page as
 * an element with an open shadow root, which Playwright's engine pierces like the page's own: on
 * a real BrowserOS page `locator('body')` matched the bar's shadow `<body>` too, a strict
 * violation. Here the engine is told that a BrowserOS UI host has no shadow root, so selectors
 * and counts, text and role matching, accessible names and the aria snapshot all read the page
 * as a browser without the bar shows it. This is the one place that rule lives.
 *
 * Every way the vendored engine enters shadow content goes through `Element.prototype.shadowRoot`
 * (selectorEvaluator, selectorUtils, roleSelectorEngine, roleUtils, ariaSnapshot). Neo's probe
 * runs in an isolated world, whose DOM prototypes are its own, so the override changes nothing
 * for the page or its scripts, which still see and use the bar.
 */
import { ProbeError } from './world'

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

let installed = false

/**
 * Hides each BrowserOS UI host's shadow root from this world's engine. Called once per world,
 * before the engine is built; a DOM without shadow roots has nothing to hide.
 */
export function hideBrowserOSUi(): void {
  if (installed) return
  let owner: object | null = Element.prototype
  let descriptor: PropertyDescriptor | undefined
  while (owner && !descriptor) {
    descriptor = Object.getOwnPropertyDescriptor(owner, 'shadowRoot')
    owner = Object.getPrototypeOf(owner)
  }
  if (!descriptor) {
    installed = true
    return
  }
  const read = descriptor.get
  if (typeof read !== 'function')
    throw new ProbeError(
      'internal',
      "this world's Element.prototype.shadowRoot is not an accessor",
    )
  Object.defineProperty(Element.prototype, 'shadowRoot', {
    configurable: true,
    enumerable: descriptor.enumerable ?? true,
    get(this: Element): ShadowRoot | null {
      return isBrowserOSUi(this) ? null : Reflect.apply(read, this, [])
    },
  })
  installed = true
}
