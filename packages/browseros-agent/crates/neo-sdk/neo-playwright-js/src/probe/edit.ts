/**
 * Edits of an element an op pinned earlier in this world. Rust sends one only after
 * `OpScope::send_effect`, because each changes the page.
 *
 *   fill           upstream's injected.fill: sets the value of an input that takes no typing
 *                  (`done`), else focuses and selects it and, once Neo's focus check passes
 *                  (keptFocus), answers `needsinput`: Rust then types the value with
 *                  Input.insertText, or presses Delete for an empty one
 *   selectOptions  upstream's injected.selectOptions: the selected values, or
 *                  `error:optionsnotfound` / `error:optionnotenabled`, which Rust retries
 *   selectText     upstream's injected.selectText
 *   focus, blur, dispatchEvent
 *   focusToType    upstream's focusNode(resetSelection) before pressSequentially's keys, then
 *                  Neo's focus check (keptFocus)
 *   setFiles       upstream's injected.setInputFiles, for setInputFiles payloads (files.ts)
 *
 * `dispatchEvent` runs here, in the isolated world, where upstream runs it in the main world.
 * The event reaches the page's listeners the same way (it is not trusted either way), and its
 * init is JSON, so no page object crosses into this world.
 */

import { drop } from './drop'
import type { FailureOutput } from './failure-output'
import { setFiles } from './files'
import type { Edit, EditRequest, NotFocused, ReplyTo, Value } from './protocol'
import type { World } from './world'

type Edited = 'error:notconnected' | Value | NotFocused

/** How each edit kind changes its element. */
function apply(
  world: World,
  edit: Edit,
  element: Element,
  output: FailureOutput,
): Edited | Promise<Edited> {
  const { injected } = world
  switch (edit.kind) {
    case 'fill': {
      // The field the text goes into: injected.fill follows a label to its control. It is
      // taken before the fill, which can make the page replace it. A value set on it directly
      // (`done`: a date, a color) needs no focus, only the field still in the document.
      const field = injected.retarget(element, 'follow-label') ?? element
      const filled = injected.fill(element, edit.value)
      if (filled === 'error:notconnected') return filled
      const typed = filled === 'needsinput' ? field : undefined
      return keptFocus(output, [element, field], typed, valueReply(filled))
    }
    case 'selectOptions': {
      const selected = injected.selectOptions(element, edit.options)
      return selected === 'error:notconnected' ? selected : valueReply(selected)
    }
    case 'selectText': {
      const selected = injected.selectText(element)
      return selected === 'done' ? valueReply(null) : selected
    }
    case 'focus':
      return doneOf(injected.focusNode(element, edit.resetSelection))
    case 'focusToType': {
      // Chromium's label hands its focus to its control, which then takes the keys.
      const typed = labelControl(element) ?? element
      const focused = injected.focusNode(element, true)
      if (focused === 'error:notconnected') return focused
      return keptFocus(output, [element, typed], typed, valueReply(null))
    }
    case 'blur':
      return doneOf(injected.blurNode(element))
    case 'dispatchEvent':
      injected.dispatchEvent(element, edit.type, edit.init)
      return valueReply(null)
    case 'drop':
      return valueReply(drop(element, edit))
    case 'setFiles':
      return valueReply(setFiles(world, element, edit.payloads))
  }
}

/**
 * Neo's check after an edit focused an element to type into it, which upstream does not make.
 * Input.insertText commits to the frame's editing selection and keys go to the focused
 * element, never to a node Rust names, so a page that moves the focus while the element takes
 * it gets the text elsewhere: a page that replaces the box from its focus listener (a framework
 * re-rendering it) leaves the selection in the box typed into before, and a page that moves the
 * focus on hands the text to the box that took it (measured on upstream 1.64.1 too).
 *
 *   one of `connected` left the document  `error:notconnected`: Rust's locator resolves
 *                                         again and finds what it matches now (the copy)
 *   `typed` does not hold the focus       `notFocused`, with what holds it: Rust fails the op
 *   else                                  `kept`: Rust types
 *
 * `typed` is the one element the text is for: the field a label names, never the label, so a
 * label's other focusable child cannot pass for it. A fill that set its value directly has
 * none, and is checked only for the field still being in the document.
 *
 * It looks after one microtask turn, so a swap a framework schedules from the focus event as a
 * microtask (React flushes a discrete event's update that way) is seen. A swap from a later
 * task can still land between this check and Rust's keys.
 */
async function keptFocus(
  output: FailureOutput,
  connected: readonly Element[],
  typed: Element | undefined,
  kept: Value,
): Promise<Edited> {
  await Promise.resolve()
  if (connected.some((element) => !element.isConnected))
    return 'error:notconnected'
  if (!typed || holdsFocus(typed)) return kept
  // Dispatch's synchronous hooks have restored before this microtask. The request owner
  // formats the holder directly, without leasing shared engine hooks across the await.
  return { kind: 'notFocused', ...holderOf(output, typed.ownerDocument) }
}

/** A `<label>`'s control, which takes the label's focus; none for any other element. */
function labelControl(element: Element): Element | null {
  if (element.localName !== 'label') return null
  const control: unknown = Reflect.get(element, 'control')
  return isElement(control) ? control : null
}

/**
 * Whether `target` holds its tree's focus: it is the focused element; or the focus is inside
 * it, where the page handed it on (a wrapper delegating to its input; a shadow host, which its
 * tree reports as focused while its own shadow tree has the focus); or, for a contenteditable's
 * child, its editing host has it.
 */
function holdsFocus(target: Element): boolean {
  // The root is a document or a shadow root; a detached subtree's root has no activeElement.
  const found: unknown = Reflect.get(target.getRootNode(), 'activeElement')
  const active = isElement(found) ? found : null
  if (!active) return false
  if (target === active || target.contains(active)) return true
  return isEditable(target) && isEditable(active) && active.contains(target)
}

function isElement(node: unknown): node is Element {
  return (
    typeof node === 'object' &&
    node !== null &&
    Reflect.get(node, 'nodeType') === Node.ELEMENT_NODE
  )
}

function isEditable(element: Element): boolean {
  return Reflect.get(element, 'isContentEditable') === true
}

/** Upstream's preview of the element that holds the document's focus, down open shadow roots;
 * none when only the document itself does (`<body>`). */
function holderOf(
  output: FailureOutput,
  document: Document | undefined,
): { holder?: string } {
  let holder = document?.activeElement ?? null
  while (holder?.shadowRoot?.activeElement)
    holder = holder.shadowRoot.activeElement
  if (
    !holder ||
    holder === document?.body ||
    holder === document?.documentElement
  )
    return {}
  return { holder: output.previewNode(holder) }
}

function valueReply(value: unknown): Value {
  return { kind: 'value', value }
}

function doneOf(result: 'error:notconnected' | 'done'): Edited {
  return result === 'done' ? valueReply(null) : result
}

function replyOf(edited: Edited): ReplyTo<'edit'> {
  return edited === 'error:notconnected' ? { kind: 'detached' } : edited
}

export function edit(
  world: World,
  request: EditRequest,
  _root: Node | undefined,
  _nodes: readonly unknown[],
  output: FailureOutput,
): ReplyTo<'edit'> | Promise<ReplyTo<'edit'>> {
  // File selection operates on the held input even when its chooser came from a
  // detached node. Other edits retain their actionability requirement.
  const element =
    request.edit.kind === 'setFiles'
      ? world.pins.held(request.pin)
      : world.pins.live(request.pin)
  if (!element) return { kind: 'detached' }
  const edited = apply(world, request.edit, element, output)
  return edited instanceof Promise ? edited.then(replyOf) : replyOf(edited)
}
