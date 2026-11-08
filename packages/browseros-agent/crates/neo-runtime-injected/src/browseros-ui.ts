/**
 * Neo: keeps BrowserOS's own in-page UI out of Neo's page reads. BrowserOS's bundled claw-app
 * mounts its human-help takeover bar on every http(s) page as `<browseros-neo-takeover>`, whose
 * shadow root holds its own `<html>`, `<body>`, input and buttons. Shipped BrowserOS builds mount
 * it open, and Playwright's selector, text and role engines and its aria snapshots enter every
 * open root through `Element.shadowRoot`, so `locator('body')` matched twice (a strict mode
 * violation), counts and `nth` took the bar's elements, and snapshots carried its nodes.
 *
 * The vendored injected script stays unchanged: Neo answers `shadowRoot` with `null` for those
 * hosts, as for a closed root, in two places only. In Neo's isolated world (`attach.ts`) the
 * getter stays replaced for the document's life; nothing but Neo runs there, and worlds share the
 * DOM, not prototypes, so the page and the extension keep the real getter. In the page's main
 * world (`call-on-element.ts`) it is replaced only around one synchronous query and restored
 * before any page listener or agent code runs. A page's own shadow roots, and every other read
 * of the bar (its rendering, its input, the accessibility tree), are untouched.
 */

/**
 * The tag names of the shadow hosts BrowserOS's bundled extensions mount in web pages: claw-app's
 * takeover bar (`apps/claw-app/entrypoints/takeover.content.ts`).
 * Deviation: `injected/src/selectorEvaluator.ts` `SelectorEvaluatorImpl._queryCSS`: `if (element.shadowRoot)` Playwright enters every open shadow root; Neo skips the root of BrowserOS's own takeover bar, which BrowserOS mounts on every page, so selectors, counts, text and role matching and aria snapshots see the page as a browser without the bar shows it.
 */
const BROWSEROS_UI_HOSTS: readonly string[] = ['browseros-neo-takeover']

/**
 * Neo: marks Neo's getter, so a later attach in the same world (another run, another Neo build)
 * finds it in place instead of wrapping it again.
 */
const HIDES_BROWSEROS_UI = '__neo_hides_browseros_ui__'

type ShadowRootGetter = (this: Element) => ShadowRoot | null

/** Neo: `get` (the getter in place), answering `null` for a BrowserOS UI host. */
function hidingGetter(get: ShadowRootGetter): ShadowRootGetter {
  const hiding = function (this: Element): ShadowRoot | null {
    // The getter in place first: it throws for a receiver that is no Element, as the native does.
    const root = get.call(this)
    return root && BROWSEROS_UI_HOSTS.includes(this.localName) ? null : root
  }
  Object.defineProperty(hiding, HIDES_BROWSEROS_UI, { value: true })
  return hiding
}

/** Neo: `Element.prototype.shadowRoot`'s descriptor in `world`, when its getter can be replaced. */
function replaceable(world: typeof globalThis): PropertyDescriptor | undefined {
  const descriptor = Object.getOwnPropertyDescriptor(
    world.Element.prototype,
    'shadowRoot',
  )
  if (!descriptor?.get || !descriptor.configurable) return undefined
  return descriptor
}

/**
 * Neo: hides BrowserOS's UI roots from every read in `world` from now on. Only for Neo's
 * isolated world; idempotent.
 */
export function hideBrowserOSUi(world: typeof globalThis): void {
  const descriptor = replaceable(world)
  if (!descriptor?.get || HIDES_BROWSEROS_UI in descriptor.get) return
  Object.defineProperty(world.Element.prototype, 'shadowRoot', {
    ...descriptor,
    get: hidingGetter(descriptor.get),
  })
}

/**
 * Neo: `read()` with BrowserOS's UI roots hidden in `world`, the page's main world, and the
 * getter restored before it returns or throws. `read` must be synchronous and must not run page
 * code (no event dispatch, no agent function): the page could otherwise see the getter. A page
 * that froze `Element.prototype` gets `read()` as Playwright runs it.
 */
export function withBrowserOSUiHidden<T>(
  world: typeof globalThis,
  read: () => T,
): T {
  const descriptor = replaceable(world)
  if (!descriptor?.get) return read()
  const prototype = world.Element.prototype
  Object.defineProperty(prototype, 'shadowRoot', {
    ...descriptor,
    get: hidingGetter(descriptor.get),
  })
  try {
    return read()
  } finally {
    Object.defineProperty(prototype, 'shadowRoot', descriptor)
  }
}
