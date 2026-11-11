/**
 * Edits of an element an op pinned earlier in this world. Rust sends one only after
 * `OpScope::send_effect`, because each changes the page.
 *
 *   fill           upstream's injected.fill: sets the value of an input that takes no typing
 *                  (`done`), else focuses and selects it (`needsinput`): Rust then types the
 *                  value with Input.insertText, or presses Delete for an empty one
 *   selectOptions  upstream's injected.selectOptions: the selected values, or
 *                  `error:optionsnotfound` / `error:optionnotenabled`, which Rust retries
 *   selectText     upstream's injected.selectText
 *   focus, blur, dispatchEvent
 *   setFiles       upstream's injected.setInputFiles, for setInputFiles payloads (files.ts)
 *
 * `dispatchEvent` runs here, in the isolated world, where upstream runs it in the main world.
 * The event reaches the page's listeners the same way (it is not trusted either way), and its
 * init is JSON, so no page object crosses into this world.
 */
import { setFiles } from './files'
import type { Edit, EditRequest, ReplyTo, Value } from './protocol'
import type { World } from './world'

type Edited = 'error:notconnected' | Value

/** How each edit kind changes its element. */
function apply(world: World, edit: Edit, element: Element): Edited {
  const { injected } = world
  switch (edit.kind) {
    case 'fill': {
      const filled = injected.fill(element, edit.value)
      return filled === 'error:notconnected' ? filled : valueReply(filled)
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
    case 'blur':
      return doneOf(injected.blurNode(element))
    case 'dispatchEvent':
      injected.dispatchEvent(element, edit.type, edit.init)
      return valueReply(null)
    case 'setFiles':
      return valueReply(setFiles(world, element, edit.payloads))
  }
}

function valueReply(value: unknown): Value {
  return { kind: 'value', value }
}

function doneOf(result: 'error:notconnected' | 'done'): Edited {
  return result === 'done' ? valueReply(null) : result
}

export function edit(world: World, request: EditRequest): ReplyTo<'edit'> {
  // File selection operates on the held input even when its chooser came from a
  // detached node. Other edits retain their actionability requirement.
  const element =
    request.edit.kind === 'setFiles'
      ? world.pins.held(request.pin)
      : world.pins.live(request.pin)
  if (!element) return { kind: 'detached' }
  const edited = apply(world, request.edit, element)
  return edited === 'error:notconnected' ? { kind: 'detached' } : edited
}
