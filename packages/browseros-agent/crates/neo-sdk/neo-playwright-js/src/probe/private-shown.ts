/**
 * `privateShown`: whether this frame's document holds a box that the mask treats as private,
 * that is not empty, and that the browser paints in clear, such as a card number or a code
 * the page declares, typed by a script or by the owner. Neo's camera asks it in every frame of
 * the tab before and after a picture, and stores no picture while one does (protected-fill
 * ruling R45). A password box paints bullets (`-webkit-text-security`), so a sign-in page keeps
 * its pictures. Read outside any mask: the box's own value only decides empty or not, and
 * nothing of it leaves this world.
 *
 * It also answers whether the document holds a private box at all, empty or not (`boxes`):
 * only such a document, or one whose private box took input, is part of the camera's reading,
 * so an ad frame's coming and going changes nothing (ruling R54). And it answers how many
 * times such a box took input since the first reading in this document. A value typed and gone again between the camera's two readings (a single-page
 * checkout that unmounts its card form) leaves both readings clear while the capture between
 * them holds it; a count that moved tells the camera to drop that capture.
 *
 * A box inside a closed shadow root is seen only once a marked snapshot has handed that root to
 * this world; until then it answers no, a named limit. Its input is never counted, even then: at
 * the document an event's path stops at the closed root's host. And a value a page's own script
 * writes with `.value =` fires no input event, so one shown and cleared again between the two
 * readings is not counted either.
 */
import { isPrivateField } from './private-field-values'
import type { Value } from './protocol'
import type { World } from './world'

/** Input events on a private box painted in clear, since the first reading in this document. */
let inputs = 0
/** Whether this document's world counts them yet: the first reading installs the listener. */
let counting = false

export function privateShown(world: World): Value {
  const { document } = world.injected
  const view = document.defaultView
  if (!counting) {
    counting = true
    // Capture phase on the document, in this isolated world: the page can neither see nor stop
    // it. The event's first target is the box itself, inside an open shadow root too.
    document.addEventListener(
      'input',
      (event) => {
        const target = event.composedPath()[0]
        if (target instanceof Element && paintedInClear(target, view))
          inputs += 1
      },
      true,
    )
  }
  const { shown, boxes } = look(document, view)
  return { kind: 'value', value: { shown, inputs, boxes } }
}

/** Whether a private box shows its value, and whether the document holds a private box at all. */
function look(
  document: Document,
  view: Window | null,
): { shown: boolean; boxes: boolean } {
  const roots: (Document | ShadowRoot)[] = [document]
  let boxes = false
  for (const root of roots) {
    for (const field of root.querySelectorAll('input, textarea')) {
      if (!isPrivateField(field, { declared: true })) continue
      boxes = true
      if (paintedInClear(field, view)) return { shown: true, boxes }
    }
    for (const host of root.querySelectorAll('*')) {
      const shadow = host.shadowRoot
      if (shadow) roots.push(shadow)
    }
  }
  return { shown: false, boxes }
}

/** A private box that holds something and shows it as typed, where it renders at all. */
function paintedInClear(field: Element, view: Window | null): boolean {
  // The page's own declarations always count here, whatever scope a caller puts around it.
  if (!isPrivateField(field, { declared: true })) return false
  const value: unknown = Reflect.get(field, 'value')
  if (typeof value !== 'string' || value === '') return false
  const security = view
    ?.getComputedStyle(field)
    .getPropertyValue('-webkit-text-security')
  if (security && security !== 'none') return false
  // A box that does not render paints nothing; one off screen still counts, as a full-page
  // picture can show it.
  const visible: unknown = Reflect.get(field, 'checkVisibility')
  return (
    typeof visible !== 'function' ||
    Reflect.apply(visible, field, [
      { visibilityProperty: true, opacityProperty: true },
    ]) === true
  )
}
