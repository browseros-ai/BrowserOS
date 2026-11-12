// fallow-ignore-file unused-file
// Rust embeds this guard in every click probe, so a target that became detached
// or disabled is refused before native input is sent. Covering content, dialogs
// and inertness are not refusals: the fallback press reaches the element itself.
// biome-ignore lint/correctness/noUnusedVariables: Rust embeds this function as a CDP functionDeclaration.
function browserosClickRefusal() {
  if (!this.isConnected) return 'Element is detached; take a new snapshot.'
  // Native inheritance includes the disabled-fieldset first-legend exception.
  if (this.matches(':disabled')) return 'Element is disabled.'
  return null
}
