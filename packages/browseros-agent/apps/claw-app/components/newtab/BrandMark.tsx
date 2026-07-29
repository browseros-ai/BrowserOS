interface BrandMarkProps {
  /** The one permitted serif-italic brand-voice touch, shown only while live. */
  statusLine?: string
}

/** Small centered BrowserClaw mark over the search bar, the calm new-tab pattern. */
export function BrandMark({ statusLine }: BrandMarkProps) {
  return (
    <div className="flex flex-col items-center gap-2">
      <img
        alt="BrowserClaw"
        className="size-9 rounded-lg shadow-card"
        src="/icons/browserclaw.svg"
      />
      {statusLine ? (
        <p className="font-serif text-ink-3 text-sm italic">{statusLine}</p>
      ) : null}
    </div>
  )
}
