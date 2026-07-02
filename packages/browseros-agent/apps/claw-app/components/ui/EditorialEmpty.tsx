interface EditorialEmptyProps {
  /** Text before the italic accent word. */
  leading: string
  /** The italic Newsreader accent word (the app's signature move). */
  accent: string
  /** Text after the italic accent word. */
  trailing: string
  /** Mono ink-3 hint line under the accent line. */
  hint: string
}

/**
 * Shared editorial empty state used across cockpit / audit / MCP.
 * No card, no lucide icon. A Newsreader italic accent line in the
 * same voice as the cockpit hero ("What are your agents *working
 * on* right now?"), followed by a mono hint. Screen-specific copy
 * lives at the call site.
 */
export function EditorialEmpty({
  leading,
  accent,
  trailing,
  hint,
}: EditorialEmptyProps) {
  return (
    <div className="py-20 text-center">
      <p className="font-extrabold text-2xl leading-tight tracking-tight md:text-3xl">
        {leading}{' '}
        <span className="font-medium font-serif text-accent italic">
          {accent}
        </span>{' '}
        {trailing}
      </p>
      <p className="mt-3 font-mono text-[12px] text-ink-3">{hint}</p>
    </div>
  )
}
