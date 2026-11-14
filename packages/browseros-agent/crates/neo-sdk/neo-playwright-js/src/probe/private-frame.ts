/**
 * The private look's question to one page above a box's frame (ruling R60): where the iframe
 * that leads down to the box draws its content, and what takes the pointer at the box's centre
 * there. Rust asks every page above, from the top document down, through `frameBox` with its
 * `look` flag; a click asks without it and keeps upstream's answers (`act.ts`).
 *
 * Two rules differ from upstream's `describeIFrameStyle` and `expectHitTarget`:
 *
 * - Placing. Upstream places no frame under any transform, an identity one included, and a
 *   payment frame commonly carries `transform: translate(0px)`. Here a frame whose content box
 *   is drawn as an upright rectangle of its own size, that is shifted and nothing else, is
 *   placed by its corner. A page that changes the frame's size or angle gets no origin, and
 *   Rust answers `cover_unread`: a guessed point would be a guessed verdict.
 * - Hitting. Upstream counts a hit on anything but the iframe and its descendants as a miss.
 *   Here what holds the frame is clear too (a wrapper, a label around the frame, a shadow host
 *   whose tree holds it, the root) when the frame lets the pointer through or is drawn there.
 *   Anything else covers. A point this page cannot show is `unseen`: outside its viewport, or
 *   where the frame is clipped away though it takes the pointer. The look has already brought
 *   the box into view at once (ruling R61), so an unseen point is a page that cannot scroll
 *   there, and Rust answers `cover_unread`.
 */
import { parentElementOrShadowHost } from '../../../neo-playwright/third_party/playwright/packages/injected/src/domUtils'
import type { Hit, Point, ReplyTo } from './protocol'
import type { World } from './world'

/** How far a matrix entry may stray from the identity's and still be read as unchanged. */
const EPSILON = 1e-6

/** `frameBox` for the look: the iframe's origin and, with `point`, the look's hit verdict. */
export function lookFrameBox(
  world: World,
  iframe: Element,
  point: Point | undefined,
): ReplyTo<'frameBox'> {
  const view = iframe.ownerDocument.defaultView
  if (!iframe.isConnected || !view) return { kind: 'detached' }
  const looked = point ? lookHit(world, iframe, point) : undefined
  return {
    kind: 'frameBox',
    origin: shiftedOrigin(view, iframe),
    hit: looked?.hit ?? null,
    unseen: looked?.unseen ?? false,
  }
}

/**
 * Where the iframe's content box starts in this page's viewport, or null when the page changes
 * the frame's size or angle. Each element from the iframe up the flat tree may shift it and
 * nothing else; the drawn box must then be the layout box's size, a check that holds whatever
 * this list of properties misses.
 */
function shiftedOrigin(view: Window, iframe: Element): Point | null {
  for (let at: Element | null = iframe; at; at = flatParent(at)) {
    if (!onlyShifts(view.getComputedStyle(at))) return null
  }
  const drawn = iframe.getBoundingClientRect()
  if (
    !(iframe instanceof HTMLElement) ||
    Math.abs(drawn.width - iframe.offsetWidth) > 1 ||
    Math.abs(drawn.height - iframe.offsetHeight) > 1
  )
    return null
  const style = view.getComputedStyle(iframe)
  // Upstream's own sums, so an untransformed frame is placed where a click places it.
  return {
    x:
      drawn.x +
      parseInt(style.borderLeftWidth || '', 10) +
      parseInt(style.paddingLeft || '', 10),
    y:
      drawn.y +
      parseInt(style.borderTopWidth || '', 10) +
      parseInt(style.paddingTop || '', 10),
  }
}

/**
 * Whether one element's style moves what it draws without changing its size or angle: a
 * `transform` that is a 2D shift (a 3D shift along z grows or shrinks under a perspective),
 * and no `scale`, `rotate`, `zoom` or motion path, which leave `transform` at `none`.
 */
function onlyShifts(style: CSSStyleDeclaration): boolean {
  if (style.scale !== 'none' && style.scale !== '') return false
  if (style.rotate !== 'none' && style.rotate !== '') return false
  if (style.zoom !== '1' && style.zoom !== '' && style.zoom !== 'normal') return false
  if (style.offsetPath !== 'none' && style.offsetPath !== '') return false
  if (style.transform === 'none' || style.transform === '') return true
  let matrix: DOMMatrixReadOnly
  try {
    matrix = new DOMMatrixReadOnly(style.transform)
  } catch {
    return false
  }
  const near = (value: number, wanted: number) => Math.abs(value - wanted) < EPSILON
  return (
    near(matrix.m11, 1) &&
    near(matrix.m12, 0) &&
    near(matrix.m13, 0) &&
    near(matrix.m14, 0) &&
    near(matrix.m21, 0) &&
    near(matrix.m22, 1) &&
    near(matrix.m23, 0) &&
    near(matrix.m24, 0) &&
    near(matrix.m31, 0) &&
    near(matrix.m32, 0) &&
    near(matrix.m33, 1) &&
    near(matrix.m34, 0) &&
    near(matrix.m43, 0) &&
    near(matrix.m44, 1)
  )
}

/**
 * Upstream's verdict, with what holds the frame made clear, and whether this page can show the
 * point at all. The topmost element is found as upstream finds it: in each tree from the
 * document down to the iframe's own, stopping where a tree's top element is not the next tree's
 * host.
 */
function lookHit(
  world: World,
  iframe: Element,
  point: Point,
): { hit: Hit; unseen: boolean } {
  const verdict = world.injected.expectHitTarget(point, iframe)
  if (verdict === 'done') return { hit: verdict, unseen: false }
  const top = topmostAt(iframe, point)
  // Nothing at all: the point lies outside this page's viewport.
  if (top === undefined) return { hit: verdict, unseen: true }
  if (!holds(top, iframe)) return { hit: verdict, unseen: false }
  // What holds the frame takes the point: the frame lets the pointer through, or is drawn there
  // under its holder, both clear; or this page clips the frame away there.
  const view = iframe.ownerDocument.defaultView
  const through = view?.getComputedStyle(iframe).pointerEvents === 'none'
  const root = iframe.getRootNode()
  const drawn =
    (root instanceof Document || root instanceof ShadowRoot) &&
    root.elementsFromPoint(point.x, point.y).includes(iframe)
  if (through || drawn) return { hit: 'done', unseen: false }
  return { hit: verdict, unseen: true }
}

function topmostAt(iframe: Element, point: Point): Element | undefined {
  const roots: (Document | ShadowRoot)[] = []
  for (let at: Element | undefined = iframe; at; ) {
    const root = at.getRootNode()
    if (root instanceof Document) {
      roots.push(root)
      break
    }
    if (!(root instanceof ShadowRoot)) break
    roots.push(root)
    at = root.host
  }
  let top: Element | undefined
  for (let index = roots.length - 1; index >= 0; index--) {
    const inner = roots[index].elementsFromPoint(point.x, point.y)[0]
    if (!inner) break
    top = inner
    const next = roots[index - 1]
    if (index && next instanceof ShadowRoot && inner !== next.host) break
  }
  return top
}

/** Whether `element` is `iframe` or holds it in the flat tree. */
function holds(element: Element, iframe: Element): boolean {
  for (let at: Element | null = iframe; at; at = flatParent(at)) {
    if (at === element) return true
  }
  return false
}

/** The element's parent in the flat tree: its slot, its parent, or its shadow root's host. */
function flatParent(element: Element): Element | null {
  return element.assignedSlot ?? parentElementOrShadowHost(element) ?? null
}
