// Every conformance page loads this first. It records what the page receives in
// window.__events, in order, so a case can return the same log under Runner P and Runner N.
// Only what a page can observe goes in: the event type, its target, trust, keys, buttons and
// values. Coordinates and timestamps differ between runs and stay out.
window.__events = []
;(() => {
  const name = (node) => {
    if (node === document) return 'document'
    if (node === window) return 'window'
    if (!(node instanceof Element)) return String(node?.nodeName)
    if (node.id) return `#${node.id}`
    return node.getAttribute('aria-label') ?? node.localName
  }
  const valueOf = (target) =>
    target.isContentEditable ? target.textContent : target.value
  const record = (event) => {
    const entry = {
      type: event.type,
      target: name(event.target),
      trusted: event.isTrusted,
    }
    if (event instanceof KeyboardEvent) {
      entry.key = event.key
      entry.code = event.code
      const held = ['alt', 'ctrl', 'meta', 'shift'].filter(
        (key) => event[`${key}Key`],
      )
      if (held.length > 0) entry.modifiers = held
    }
    if (event instanceof MouseEvent) {
      entry.button = event.button
      entry.detail = event.detail
    }
    if (event.type === 'input' || event.type === 'change')
      entry.value = valueOf(event.target)
    window.__events.push(entry)
  }
  for (const type of [
    'pointerdown',
    'mousedown',
    'pointerup',
    'mouseup',
    'click',
    'dblclick',
    'contextmenu',
    'keydown',
    'keyup',
    'input',
    'change',
    'focus',
    'blur',
    'submit',
  ])
    document.addEventListener(type, record, true)
  document.addEventListener('DOMContentLoaded', record)
  window.addEventListener('load', record)
})()
