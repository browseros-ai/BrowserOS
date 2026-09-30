// biome-ignore-all lint: Injected function is called with the resolved DOM node as this.
// Validate and select only. The Rust caller owns native input and final readback;
// assigning value or dispatching synthetic events here would bypass the editor.
function browserosPrepareFill(mode) {
  if (!this.isConnected) return 'Element is detached. Take a new snapshot.'
  const input = this.tagName === 'INPUT'
  const textControl = input || this.tagName === 'TEXTAREA'
  if (
    input &&
    !['text', 'search', 'email', 'tel', 'url', 'password', 'number'].includes(
      this.type,
    )
  ) {
    return 'This input type does not support text fill.'
  }
  if (!textControl && !this.isContentEditable) {
    return 'Element is not an editable text field.'
  }
  if (this.matches(':disabled') || this.readOnly) {
    return 'Element is disabled or read-only.'
  }
  const style = this.ownerDocument.defaultView.getComputedStyle(this)
  if (
    !this.getClientRects().length ||
    style.visibility === 'hidden' ||
    style.visibility === 'collapse'
  ) {
    return 'Element is not visible.'
  }
  if (mode === 'validate') return true
  if (this.getRootNode().activeElement !== this) {
    return 'Element did not receive focus.'
  }
  if (mode === 'focused') return true
  let collapseToEnd = false
  if (textControl) {
    if (mode === 'replace') {
      this.select()
    } else {
      // Email/number can select all but expose no JS caret API. The caller
      // collapses that native selection with ArrowRight before inserting.
      if (this.selectionStart === null) {
        this.select()
        collapseToEnd = true
      } else {
        this.setSelectionRange(this.value.length, this.value.length)
      }
    }
  } else {
    const range = this.ownerDocument.createRange()
    range.selectNodeContents(this)
    if (mode === 'append') range.collapse(false)
    const selection = this.ownerDocument.defaultView.getSelection()
    selection.removeAllRanges()
    selection.addRange(range)
  }
  // Selection listeners can detach the field or redirect focus.
  return this.isConnected && this.getRootNode().activeElement === this
    ? collapseToEnd
      ? 'collapseToEnd'
      : true
    : 'Element lost focus while preparing input. Take a new snapshot.'
}
