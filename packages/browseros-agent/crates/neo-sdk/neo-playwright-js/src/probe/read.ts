/**
 * Reads: `read` resolves one element and reads it in the same call, `readAll` reads every
 * match. Both run in the sealed world, so a value comes back through the seal: a password,
 * card or one-time-code field reads `••••`. Neither waits; Rust's retry loop does.
 *
 * Each read is upstream's callback from server/frames.ts at v1.64.1 (`textContent`,
 * `innerText`, `inputValue` and the rest), with the same checks and the same thrown
 * messages, run here instead of through `injected.eval`.
 */
import { parentElementOrShadowHost } from '../../../neo-playwright/third_party/playwright/packages/injected/src/domUtils'
import type {
  Checkable,
  FileInput,
  FrameOrigin,
  Located,
  PinnedMatch,
  ReadAllRequest,
  ReadRequest,
  Rect,
  ReplyTo,
} from './protocol'
import { queryAll, resolveTarget } from './resolve'
import { frameSource, ProbeError, type World } from './world'

/** A read's value, and the element it was read from (a label's control for `inputValue`). */
type Reading = { value: unknown; from: Element }
type Reader = (world: World, element: Element) => Reading

const XHTML = 'http://www.w3.org/1999/xhtml'

/** How each read kind reads its element. */
function readerFor(request: ReadRequest, root: Node | undefined): Reader {
  const { read, frameVisible } = request
  switch (read.kind) {
    case 'textContent':
      return (_world, element) => ({
        value: element.textContent,
        from: element,
      })
    case 'innerText':
      return (world, element) => {
        if (element.namespaceURI !== XHTML)
          throw world.injected.createStacklessError(
            'Node is not an HTMLElement',
          )
        return { value: (element as HTMLElement).innerText, from: element }
      }
    case 'innerHtml':
      return (_world, element) => ({ value: element.innerHTML, from: element })
    case 'attribute':
      return (_world, element) => ({
        value: element.getAttribute(read.name),
        from: element,
      })
    case 'inputValue':
      return inputValue
    case 'state':
      // Upstream's `elementState`: `visible` and `hidden` read the element itself and its
      // frames' visibility, the others follow a label to its control and throw on an element
      // the state does not apply to ("Not a checkbox or radio button").
      return (world, element) => ({
        value: world.injected.elementState(element, read.state, frameVisible)
          .matches,
        from: element,
      })
    case 'boundingBox':
      return (_world, element) => ({ value: borderBox(element), from: element })
    case 'frameOrigin':
      return (_world, element) => ({
        value: frameOrigin(element),
        from: element,
      })
    case 'located':
      return (world, element) => ({
        value: located(world, request, element, root),
        from: element,
      })
    case 'checkable':
      return (world, element) => ({
        value: checkable(world, element),
        from: element,
      })
    case 'fileInput':
      return (world, element) => fileInput(world, element, read)
  }
}

/**
 * Upstream's `_setInputFiles` checks (server/dom.ts at v1.64.1), in its order and with its
 * messages: the element, or the control its label names, must be an `<input>`; several files
 * need `multiple` or `webkitdirectory`; a directory needs `webkitdirectory`, and nothing else
 * may go to such an input. `type=file` is not checked here, as upstream does not: Chromium
 * refuses paths for any other input, and upstream's injected script leaves one alone.
 */
function fileInput(
  world: World,
  element: Element,
  read: { multiple: boolean; directory: boolean },
): Reading {
  const { injected } = world
  const control = injected.retarget(element, 'follow-label')
  if (control?.tagName !== 'INPUT')
    throw injected.createStacklessError('Node is not an HTMLInputElement')
  const input = control as HTMLInputElement
  if (read.multiple && !input.multiple && !input.webkitdirectory)
    throw injected.createStacklessError(
      'Non-multiple file input can only accept single file',
    )
  if (read.directory && !input.webkitdirectory)
    throw injected.createStacklessError(
      'File input does not support directories, pass individual files instead',
    )
  if (!read.directory && input.webkitdirectory)
    throw injected.createStacklessError(
      '[webkitdirectory] input requires passing a path to a directory',
    )
  const pin = world.pins.add(input)
  const value: FileInput = {
    pin,
    target: element === input ? pin : world.pins.add(element),
    takesPaths: input.type === 'file',
  }
  return { value, from: input }
}

/**
 * Upstream's `_setChecked` check: `elementState(el, 'checked')` follows a label to its control
 * and throws "Not a checkbox or radio button" for anything else; `isRadio` is that control's.
 */
function checkable(world: World, element: Element): Checkable {
  const { matches, isRadio } = world.injected.elementState(element, 'checked')
  return { checked: matches, radio: isRadio === true }
}

/** Upstream's `inputValue`: the control a label names, and only a form control. */
function inputValue(world: World, element: Element): Reading {
  const control = world.injected.retarget(element, 'follow-label')
  if (
    !control ||
    (control.nodeName !== 'INPUT' &&
      control.nodeName !== 'TEXTAREA' &&
      control.nodeName !== 'SELECT')
  )
    throw world.injected.createStacklessError(
      'Node is not an <input>, <textarea> or <select> element',
    )
  return { value: (control as HTMLInputElement).value, from: control }
}

/**
 * The border box in this frame's viewport, as `DOM.getBoxModel` gives upstream: null for an
 * element with no layout box (`display: none`, `display: contents`), a box for one that is
 * only `visibility: hidden`.
 */
function borderBox(element: Element): Rect | null {
  if (element.getClientRects().length === 0) return null
  const { x, y, width, height } = element.getBoundingClientRect()
  return { x, y, width, height }
}

/**
 * Where an `<iframe>`'s document starts in this frame's viewport, and its scale there. Without
 * transforms it is the border box plus border and padding, as upstream's
 * `_checkFrameIsHitTarget` places a point in a child frame (`describeIFrameStyle`). Under a
 * scale or translate, on the iframe or any ancestor, the rendered border box over the layout
 * size gives the scale, and the inset scales with it. A rotate, skew, flip or 3D transform
 * leaves no such box: `rotated`. Upstream ignores transforms here, which places such a box
 * wrongly instead of refusing it.
 */
function frameOrigin(element: Element): FrameOrigin | null {
  if (frameSource(element) === undefined)
    throw new ProbeError('internal', 'frameOrigin reads an <iframe> or <frame>')
  const view = element.ownerDocument.defaultView
  const box = borderBox(element)
  if (!box || !view || !element.isConnected) return null
  const transform = transformOf(element, view)
  if (transform === 'rotated') return 'rotated'
  const style = view.getComputedStyle(element)
  const px = (value: string) => Number.parseFloat(value) || 0
  const left = px(style.borderLeftWidth) + px(style.paddingLeft)
  const top = px(style.borderTopWidth) + px(style.paddingTop)
  const insets = {
    width: left + px(style.borderRightWidth) + px(style.paddingRight),
    height: top + px(style.borderBottomWidth) + px(style.paddingBottom),
  }
  // The used `width` and `height` are the content box's, or the border box's under
  // `box-sizing: border-box`; either way untransformed.
  const contentBox = style.boxSizing === 'border-box' ? 0 : 1
  const layoutWidth = px(style.width) + contentBox * insets.width
  const layoutHeight = px(style.height) + contentBox * insets.height
  // Without a transform the scale is exactly 1: the computed size is rounded where layout is
  // not, and their ratio would put a few millionths of a pixel into every box.
  const scaled = transform === 'scaled'
  const scaleX = scaled && layoutWidth > 0 ? box.width / layoutWidth : 1
  const scaleY = scaled && layoutHeight > 0 ? box.height / layoutHeight : 1
  return {
    x: box.x + left * scaleX,
    y: box.y + top * scaleY,
    scaleX,
    scaleY,
  }
}

/**
 * What the transforms on `element` and its ancestors (through shadow hosts, as
 * `describeIFrameStyle` walks) do to it: nothing, only scale and move it (`scaled`), or more
 * (`rotated`): a rotate, skew, flip or 3D matrix, or the `rotate` and `scale` properties that
 * `transform` does not show.
 */
function transformOf(
  element: Element,
  view: Window,
): 'none' | 'scaled' | 'rotated' {
  let found: 'none' | 'scaled' = 'none'
  for (
    let at: Element | undefined = element;
    at;
    at = parentElementOrShadowHost(at)
  ) {
    const style = view.getComputedStyle(at)
    if (style.rotate !== 'none') return 'rotated'
    if (style.scale !== 'none') {
      const factors = style.scale.split(' ').map(Number.parseFloat)
      if (factors.some((factor) => !(factor > 0))) return 'rotated'
      found = 'scaled'
    }
    if (style.translate !== 'none') found = 'scaled'
    if (style.transform === 'none') continue
    const matrix = new DOMMatrixReadOnly(style.transform)
    if (
      !matrix.is2D ||
      matrix.b !== 0 ||
      matrix.c !== 0 ||
      matrix.a <= 0 ||
      matrix.d <= 0
    )
      return 'rotated'
    found = 'scaled'
  }
  return found
}

/**
 * What upstream's `waitForSelector` callback reports on one poll: the first match, pinned so
 * Rust can make a handle of it, whether it is visible (its frames counted), and how many
 * matched, for the "resolved to N elements" call-log line of a non-strict wait.
 */
function located(
  world: World,
  request: ReadRequest,
  element: Element,
  root: Node | undefined,
): Located {
  const count =
    request.strictness === 'strict'
      ? 1
      : queryAll(world, request.target, root).length
  return {
    pin: world.pins.add(element),
    preview: world.injected.previewNode(element),
    visible: world.injected.elementState(
      element,
      'visible',
      request.frameVisible,
    ).matches,
    count,
  }
}

export function read(
  world: World,
  request: ReadRequest,
  root: Node | undefined,
): ReplyTo<'read'> {
  const reader = readerFor(request, root)
  // File choosers may originate from an input that was never attached. Upstream's
  // setInputFiles checks its type and properties directly, without requiring attachment.
  const found =
    request.read.kind === 'fileInput' &&
    request.target === 'root' &&
    root instanceof Element
      ? { kind: 'one' as const, element: root }
      : resolveTarget(world, request.target, request.strictness, root)
  if (found.kind !== 'one') return found
  const { value, from } = reader(world, found.element)
  return { kind: 'read', value, sensitive: world.seal.kind(from) }
}

/**
 * Every match now. The text reads are upstream's client `$$eval`s (run in the main world
 * there, through the seal here): `textContent || ''` and `innerText`, one per match in
 * document order; an element without `innerText` (SVG) reads as ''.
 */
export function readAll(
  world: World,
  request: ReadAllRequest,
  root: Node | undefined,
): ReplyTo<'readAll'> {
  const elements = queryAll(world, request.target, root)
  const count = elements.length
  switch (request.read) {
    case 'count':
      return { kind: 'values', count, values: [] }
    case 'pins': {
      // One entry for the whole batch: Rust makes a handle of each before any can be evicted.
      const pins = world.pins.addAll(elements)
      return {
        kind: 'values',
        count,
        values: elements.map(
          (element, index): PinnedMatch => ({
            pin: pins[index] ?? 0,
            preview: world.injected.previewNode(element),
          }),
        ),
      }
    }
    case 'textContents':
      return {
        kind: 'values',
        count,
        values: elements.map((element) => element.textContent ?? ''),
      }
    case 'innerTexts':
      return {
        kind: 'values',
        count,
        values: elements.map(
          (element) => (element as HTMLElement).innerText ?? '',
        ),
      }
  }
}
