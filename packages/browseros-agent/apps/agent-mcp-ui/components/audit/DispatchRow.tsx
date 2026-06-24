import { AlertTriangle, ChevronDown, ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { cn } from '@/lib/utils'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import { hexForSlug } from '@/screens/audit/audit.colors'
import {
  formatRelative,
  parseResultMeta,
  siteOf,
} from '@/screens/audit/audit.helpers'

interface DispatchRowProps {
  row: ToolDispatchRow
  now: number
}

/**
 * One row per audit-log entry. Header shows agent dot + label, tool
 * name, site host, relative timestamp, error indicator. Click to
 * expand and reveal raw args + result meta + the session id (useful
 * for grepping the cockpit log).
 */
export function DispatchRow({ row, now }: DispatchRowProps) {
  const [expanded, setExpanded] = useState(false)
  const meta = parseResultMeta(row.resultMeta)
  const color = hexForSlug(row.slug)
  const isError = meta?.isError === true

  return (
    <div
      className={cn(
        'border-border-2 border-b transition last:border-b-0',
        isError ? 'bg-red-tint/40' : 'hover:bg-bg-sunken/60',
      )}
    >
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center gap-3 px-4 py-2.5 text-left"
      >
        {expanded ? (
          <ChevronDown className="size-3.5 shrink-0 text-ink-3" />
        ) : (
          <ChevronRight className="size-3.5 shrink-0 text-ink-3" />
        )}
        <span
          aria-hidden
          className="size-2 shrink-0 rounded-full"
          style={{ backgroundColor: color }}
        />
        <span className="w-32 shrink-0 truncate font-bold text-[12.5px]">
          {row.agentLabel}
        </span>
        <span className="w-20 shrink-0 font-mono text-[11.5px] text-ink-2">
          {row.toolName}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-ink-3">
          {siteOf(row.url)}
        </span>
        {isError && (
          <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-red-tint px-2 py-0.5 font-bold text-[10.5px] text-red">
            <AlertTriangle className="size-3" />
            error
          </span>
        )}
        {typeof row.durationMs === 'number' && (
          <span className="w-12 shrink-0 text-right font-mono text-[11px] text-ink-4">
            {row.durationMs}ms
          </span>
        )}
        <span className="w-16 shrink-0 text-right text-[11px] text-ink-4">
          {formatRelative(row.createdAt, now)}
        </span>
      </button>
      {expanded && (
        <div className="grid grid-cols-2 gap-4 px-4 pb-3 pl-12 text-[12px]">
          <div>
            <div className="mb-1 font-semibold text-[10.5px] text-ink-3 uppercase tracking-wider">
              args
            </div>
            <pre className="overflow-x-auto rounded-md bg-ink p-2 font-mono text-[11px] text-card">
              {prettyJson(row.argsJson)}
            </pre>
          </div>
          <div>
            <div className="mb-1 font-semibold text-[10.5px] text-ink-3 uppercase tracking-wider">
              result
            </div>
            <pre className="overflow-x-auto rounded-md bg-ink p-2 font-mono text-[11px] text-card">
              {prettyJson(row.resultMeta)}
            </pre>
            <div className="mt-2 text-ink-3 text-xs">
              session{' '}
              <span className="font-mono text-ink-2">
                {row.sessionId.slice(0, 8)}...
              </span>
              {row.url && (
                <>
                  {' . '}
                  <a
                    href={row.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="font-mono text-accent-ink hover:underline"
                  >
                    open
                  </a>
                </>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

function prettyJson(raw: string | null): string {
  if (!raw) return '{}'
  try {
    return JSON.stringify(JSON.parse(raw), null, 2)
  } catch {
    return raw
  }
}
