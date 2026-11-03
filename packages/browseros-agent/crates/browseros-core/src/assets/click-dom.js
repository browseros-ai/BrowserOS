// fallow-ignore-file unused-file
// Rust embeds this function with include_str!; JavaScript imports do not reach it.
// It stands in for BrowserOS's DOM.click on builds without it: the same refusal
// codes, then focus and the DOM's own click(), which reaches the element through
// anything covering it. Its events are untrusted.
// biome-ignore lint/correctness/noUnusedVariables: Rust embeds this function as a CDP functionDeclaration.
function browserosActivateDom() {
  const element =
    this.nodeType === Node.ELEMENT_NODE ? this : this.parentElement
  const refusal = () => {
    if (!element?.isConnected) return 'detached'
    if (!element.checkVisibility({ checkVisibilityCSS: true }))
      return 'notVisible'
    if (element.matches(':disabled')) return 'disabled'
    return null
  }
  let refused = refusal()
  if (refused) return { refused }
  if (typeof element.click !== 'function') return { refused: 'unsupported' }
  element.focus?.({ preventScroll: true })
  // Focus handlers can remove, hide or disable the element; never click it then.
  refused = refusal()
  if (refused) return { refused }
  element.click()
  // This acknowledges dispatch only. Application state is the caller's success check.
  return {}
}
