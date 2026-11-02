// fallow-ignore-file unused-file
// Rust embeds this function with include_str!; JavaScript imports do not reach it.
// biome-ignore lint/correctness/noUnusedVariables: Rust embeds this function as a CDP functionDeclaration.
function browserosActivateDom() {
  if (!this.isConnected) return 'Element is detached; take a new snapshot.'
  // Let the browser apply native inheritance, including the first-legend exception.
  if (this.matches(':disabled'))
    return 'Element is disabled or inert; DOM activation was not sent.'
  for (
    let node = this;
    node;
    node = node.parentElement || node.getRootNode().host
  ) {
    if (node.matches('[aria-disabled="true"], [inert]')) {
      return 'Element is disabled or inert; DOM activation was not sent.'
    }
  }
  if (typeof this.click !== 'function')
    return 'Element does not support DOM activation.'
  this.click()
  // This acknowledges dispatch only. Application state is the caller's success check.
  return null
}
