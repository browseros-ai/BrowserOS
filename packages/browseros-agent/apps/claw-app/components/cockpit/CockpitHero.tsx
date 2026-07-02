/**
 * Cockpit hero. Editorial-cockpit direction: compressed vertical
 * space, left-aligned, Newsreader italic accent on "working on"
 * preserved as the app signature. Filters live in the section
 * header below, not here.
 */
export function CockpitHero() {
  return (
    <header className="pt-1">
      <h1 className="font-extrabold text-3xl leading-[1.15] tracking-tight md:text-4xl">
        What are your agents{' '}
        <span className="font-medium font-serif text-accent italic">
          working on
        </span>{' '}
        right now?
      </h1>
    </header>
  )
}
