interface AuditEmptyProps {
  variant: 'search-miss' | 'zero-tasks' | 'error'
}

const COPY = {
  'search-miss': {
    leading: 'nothing',
    accent: 'matches',
    trailing: 'these filters.',
    hint: 'Try clearing them or widening the search window.',
  },
  'zero-tasks': {
    leading: 'the audit is',
    accent: 'quiet',
    trailing: 'so far.',
    hint: 'Connect an agent from the MCP page and run a tool. Sessions land here within a few seconds.',
  },
  error: {
    leading: 'could not',
    accent: 'load',
    trailing: 'the audit.',
    hint: 'Check that the cockpit server is running and the audit database is reachable.',
  },
} as const

/**
 * Editorial empty state for the audit list. No card, no icon. A
 * Newsreader italic accent line in the same voice as the cockpit
 * hero ("What are your agents *working on* right now?"), followed
 * by a mono hint. Three variants: search-miss, zero-tasks, error.
 */
export function AuditEmpty({ variant }: AuditEmptyProps) {
  const copy = COPY[variant]
  return (
    <div className="py-20 text-center">
      <p className="font-extrabold text-2xl leading-tight tracking-tight md:text-3xl">
        {copy.leading}{' '}
        <span className="font-medium font-serif text-accent italic">
          {copy.accent}
        </span>{' '}
        {copy.trailing}
      </p>
      <p className="mt-3 font-mono text-[12px] text-ink-3">{copy.hint}</p>
    </div>
  )
}
