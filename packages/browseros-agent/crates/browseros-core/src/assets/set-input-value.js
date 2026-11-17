// biome-ignore-all lint: Injected function is called with the resolved DOM node as this.
// Sets a date, time, colour or range input, which has no text editor to type into. Runs in
// the isolated world, so the assignment reaches the native setter rather than a framework's
// main-world wrapper, and the input event that follows lets the framework see the change.
function browserosSetInputValue(value) {
  if (!this.isConnected) return 'Element is detached. Take a new snapshot.'
  const wanted = value.trim()
  const previous = this.value
  this.focus()
  this.value = wanted
  // The browser stores a value in its own form: a datetime drops :00 seconds, a colour is
  // lowercased or falls back to #000000, a range clamps to its bounds. The caller reads the
  // field back afterwards, so what it stored is the result. The one refusal the browser
  // signals is a date or time it cannot parse, which empties the field (the HTML value
  // sanitization rule). Put the old value back and fire nothing, so the page never sees the
  // field cleared.
  if (wanted !== '' && this.value === '') {
    this.value = previous
    const formats = {
      date: 'YYYY-MM-DD',
      time: 'HH:MM',
      'datetime-local': 'YYYY-MM-DDTHH:MM',
      month: 'YYYY-MM',
      week: 'YYYY-Www',
    }
    return `"${wanted}" is not a valid ${this.type} value. Use ${formats[this.type] ?? 'the format the field expects'}.`
  }
  this.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
  this.dispatchEvent(new Event('change', { bubbles: true }))
  return true
}
