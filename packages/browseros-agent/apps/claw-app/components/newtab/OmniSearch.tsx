import { Search } from 'lucide-react'
import { useState } from 'react'
import { submitOmni } from './newtab-search.helpers'

const PLACEHOLDER = 'Search the web or type a URL'

/**
 * Chrome-style omnibox. Always acts on the current tab: a URL navigates,
 * anything else runs a web search. Rendered identically in both new-tab
 * states; only its position in the column changes.
 */
export function OmniSearch() {
  const [value, setValue] = useState('')
  return (
    <form
      className="w-full"
      onSubmit={(event) => {
        event.preventDefault()
        submitOmni(value)
      }}
    >
      <div className="relative flex items-center">
        <Search
          aria-hidden
          className="pointer-events-none absolute left-4 size-4 text-ink-3"
        />
        <input
          // biome-ignore lint/a11y/noAutofocus: the omnibox is the primary action of the new tab; focus lands on load so the user can type immediately.
          autoFocus
          type="text"
          value={value}
          onChange={(event) => setValue(event.target.value)}
          aria-label={PLACEHOLDER}
          placeholder={PLACEHOLDER}
          spellCheck={false}
          autoComplete="off"
          className="h-12 w-full rounded-2xl border border-border-2 bg-card pr-4 pl-11 text-ink text-sm shadow-card outline-none transition-colors placeholder:text-ink-3 focus:border-accent focus:ring-2 focus:ring-ring"
        />
      </div>
    </form>
  )
}
