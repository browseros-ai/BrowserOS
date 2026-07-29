import { Search } from 'lucide-react'
import { useState } from 'react'
import { submitOmni } from './newtab-search.helpers'

/**
 * The calm new-tab omnibox: one pill that routes a URL to the current tab or a
 * query to the browser's default search engine. Autofocused because searching
 * is the page's primary action, matching the browser new-tab pattern.
 */
export function OmniSearch() {
  const [value, setValue] = useState('')

  return (
    <form
      aria-label="Search the web or type a URL"
      className="w-full max-w-2xl"
      onSubmit={(event) => {
        event.preventDefault()
        submitOmni(value)
      }}
    >
      <div className="flex items-center gap-3 rounded-2xl border border-border bg-card px-4 py-3 shadow-card transition-colors focus-within:border-accent focus-within:ring-2 focus-within:ring-ring">
        <Search aria-hidden className="size-5 shrink-0 text-ink-3" />
        <input
          // biome-ignore lint/a11y/noAutofocus: the new-tab omnibox is the page's primary action; focusing it on load matches the browser new-tab pattern
          autoFocus
          aria-label="Search the web or type a URL"
          autoComplete="off"
          className="min-w-0 flex-1 bg-transparent text-base text-ink placeholder:text-ink-3 focus:outline-none"
          enterKeyHint="go"
          onChange={(event) => setValue(event.target.value)}
          placeholder="Search the web or type a URL"
          spellCheck={false}
          type="text"
          value={value}
        />
      </div>
    </form>
  )
}
