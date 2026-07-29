import { Search } from 'lucide-react'
import { useRef } from 'react'
import { runOmniSearch } from './newtab-search.helpers'

const PLACEHOLDER = 'Search the web or type a URL'

/** Chrome-style omnibox: routes a URL-looking entry to navigation, else search. */
export function OmniSearch() {
  const inputRef = useRef<HTMLInputElement>(null)

  return (
    <form
      className="w-full max-w-2xl"
      onSubmit={(event) => {
        event.preventDefault()
        runOmniSearch(inputRef.current?.value ?? '')
      }}
    >
      <div className="relative flex items-center">
        <Search
          aria-hidden
          className="pointer-events-none absolute left-4 size-4 text-ink-3"
        />
        <input
          ref={inputRef}
          // biome-ignore lint/a11y/noAutofocus: search is the new-tab's primary action, mirroring the browser omnibox that focuses on a fresh tab
          autoFocus
          type="text"
          name="omnisearch"
          aria-label={PLACEHOLDER}
          placeholder={PLACEHOLDER}
          autoComplete="off"
          spellCheck={false}
          className="h-12 w-full rounded-2xl border border-border-2 bg-card pr-4 pl-11 text-ink text-sm shadow-card outline-none transition-[border-color,box-shadow] placeholder:text-ink-3 focus:border-accent focus:ring-2 focus:ring-ring/35"
        />
      </div>
    </form>
  )
}
