// fallow-ignore-file unused-file
// Rust embeds this function with include_str!; JavaScript imports do not reach it.
// biome-ignore lint/correctness/noUnusedVariables: Rust embeds this function as a CDP functionDeclaration.
function browserosActivateDom() {
  if (typeof this.click !== 'function')
    return { refused: 'Element does not support DOM activation.' }
  this.click()
  // This acknowledges dispatch only. Application state is the caller's success check.
  return {}
}
