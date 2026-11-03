// fallow-ignore-file unused-file
// Rust embeds this shared guard in probes and the one-shot activation function.
// biome-ignore lint/correctness/noUnusedVariables: Rust embeds this function as a CDP functionDeclaration.
function browserosClickRefusal() {
  if (!this.isConnected) return 'Element is detached; take a new snapshot.'
  // Native inheritance includes the disabled-fieldset first-legend exception.
  if (this.matches(':disabled')) return 'Element is disabled or inert.'
  const up = (node) => node.parentElement || node.getRootNode().host
  const contains = (ancestor, child) => {
    for (let node = child; node; node = up(node))
      if (node === ancestor) return true
    return false
  }
  // Same-process frames share a CDP session. Include their owner elements;
  // the Rust transaction separately checks owners across OOPIF endpoints.
  for (let element = this; element; ) {
    for (let node = element; node; node = up(node))
      if (node.matches('[aria-disabled="true"], [inert]'))
        return 'Element is disabled or inert.'
    const roots = [element.ownerDocument]
    for (let index = 0; index < roots.length; index++) {
      // ARIA modals are application-owned, so Chromium may still hit-test their
      // background. Respect visible modal declarations, including open roots.
      for (const modal of roots[index].querySelectorAll(
        '[aria-modal="true"]',
      )) {
        const style = modal.ownerDocument.defaultView.getComputedStyle(modal)
        if (
          style.visibility === 'visible' &&
          modal.getClientRects().length &&
          !contains(modal, element)
        )
          return 'Element is outside the active modal; handle the modal first.'
      }
      for (const node of roots[index].querySelectorAll('*'))
        if (node.shadowRoot) roots.push(node.shadowRoot)
    }
    element = element.ownerDocument.defaultView.frameElement
  }
  return null
}
