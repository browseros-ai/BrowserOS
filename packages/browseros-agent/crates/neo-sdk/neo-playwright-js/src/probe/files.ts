/**
 * The probe's half of `setInputFiles` that changes the page or waits on it; read.ts makes
 * upstream's checks first. Payloads and `[]` are set here by upstream's own injected
 * `setInputFiles`. A directory is set by Rust's `DOM.setFileInputFiles`, around which this file
 * arms and reads upstream's wait for the `input` event.
 */
import type {
  ArmInputRequest,
  FilePayload,
  FilesSet,
  InputArrivedRequest,
  ReplyTo,
} from './protocol'
import type { World } from './world'

/**
 * Elements a directory upload armed, with whether their `input` came. Module state is this
 * world's: the bundle is evaluated once per isolated world.
 */
const armed = new WeakMap<Element, { arrived: boolean }>()

/**
 * Upstream's injected `setInputFiles(node, payloads)` from this world: Files built from each
 * payload's base64 (`lastModified` now, since a payload carries none) set through a
 * DataTransfer, then `input` (bubbles, composed) and `change` (bubbles), all untrusted. A
 * non-file input is left alone without an error, since upstream ignores the reason the call
 * returns. What the call throws comes back as CDP would describe it, the text upstream's error
 * shows: an isolated world's `atob` throws a DOMException on text that is not base64, which
 * CDP describes as `DOMException` (measured on BrowserOS neo, Chrome 155).
 */
export function setFiles(
  world: World,
  element: Element,
  payloads: FilePayload[],
): FilesSet {
  try {
    world.injected.setInputFiles(element, payloads)
    return null
  } catch (error) {
    return { threw: described(error) }
  }
}

function described(error: unknown): string {
  if (typeof DOMException === 'function' && error instanceof DOMException)
    return 'DOMException'
  if (error instanceof Error)
    return String(error.stack ?? error).split('\n')[0] ?? error.name
  return String(error)
}

/**
 * Arms the wait upstream starts right before it sets a directory: a once-listener for `input`
 * on the element the locator or handle named, in this world, where the page cannot see it.
 * Upstream's listener holds its element whether or not it stays in the document, so a pin
 * that left its document still arms.
 */
export function armInput(
  world: World,
  request: ArmInputRequest,
): ReplyTo<'armInput'> {
  const element = world.pins.held(request.pin)
  if (!element) return { kind: 'detached' }
  const state = { arrived: false }
  armed.set(element, state)
  element.addEventListener(
    'input',
    () => {
      state.arrived = true
    },
    { once: true },
  )
  return { kind: 'value', value: null }
}

/** Whether the `input` armInput listened for has come. */
export function inputArrived(
  world: World,
  request: InputArrivedRequest,
): ReplyTo<'inputArrived'> {
  const element = world.pins.held(request.pin)
  if (!element) return { kind: 'detached' }
  return { kind: 'value', value: armed.get(element)?.arrived === true }
}
