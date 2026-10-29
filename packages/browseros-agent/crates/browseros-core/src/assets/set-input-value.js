// biome-ignore-all lint: Injected function is called with the resolved DOM node as this.
// Sets a date, time, colour or range input, which has no text editor to type into. Runs in
// the isolated world, so the assignment reaches the native setter rather than a framework's
// main-world wrapper, and the input event that follows lets the framework see the change.
function browserosSetInputValue(value) {
  if (!this.isConnected) return 'Element is detached. Take a new snapshot.'
  const formats = {
    date: 'YYYY-MM-DD',
    time: 'HH:MM',
    'datetime-local': 'YYYY-MM-DDTHH:MM',
    month: 'YYYY-MM',
    week: 'YYYY-Www',
    color: '#rrggbb',
    range: `a number from ${this.min || 0} to ${this.max || 100}`,
  }
  const wanted = value.trim()
  const previous = this.value
  this.focus()
  this.value = wanted
  // The browser drops a value its control cannot hold, which clears the field, and lowercases
  // a colour. A refused value puts the old one back, so an error leaves the field as it was.
  if (this.value.toLowerCase() !== wanted.toLowerCase()) {
    this.value = previous
    return `"${wanted}" is not a valid ${this.type} value. Use ${formats[this.type]}.`
  }
  this.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
  this.dispatchEvent(new Event('change', { bubbles: true }))
  return true
}
