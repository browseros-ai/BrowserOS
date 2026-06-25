/**
 * Cockpit hero. Lifted verbatim from the design prototype's dashboard:
 * sans-serif title with a Newsreader italic accent on "working on"
 * sitting on the BrowserOS accent color. Subtitle anchors the IA
 * ("watch, approve, audit") so the user doesn't expect the cockpit
 * to be the place they author tasks.
 */
export function CockpitHero() {
  return (
    <header className="space-y-3 pt-2 text-center">
      <h1 className="font-extrabold text-4xl leading-tight tracking-tight md:text-5xl">
        What are your agents{' '}
        <span className="font-medium font-serif text-accent italic">
          working on
        </span>{' '}
        right now?
      </h1>
      <p className="text-ink-3 text-sm">
        Tasks start in Claude Code &amp; Codex. BrowserOS is where you watch,
        approve, and audit them.
      </p>
    </header>
  )
}
