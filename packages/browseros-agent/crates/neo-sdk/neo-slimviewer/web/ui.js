// The two facts host's takeover pill (host-pill.js) reads from noVNC's `UI` object, answered by
// slimviewer.html, so host can load its pill files unchanged beside this page.
const page = () => window.slimviewer || {}
// Re-export from host-clipboard.js; checked results pass through, only tap-again is renamed.
// Call through immediately: deferring paste() would lose the trusted phone tap.
export function pasteFromPill() {
  if (!page().connected || page().viewOnly) return Promise.resolve('inactive')
  return page().paste().then(outcome => outcome === 'tap-again' ? 'cancelled' : outcome)
}
export default {
  get connected() {
    return Boolean(page().connected)
  },
  get rfb() {
    return page().connected ? { viewOnly: Boolean(page().viewOnly) } : null
  },
}
