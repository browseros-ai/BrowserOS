import { Loader2 } from 'lucide-react'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import { cn } from '@/lib/utils'
import type { ConnectionState } from '@/modules/api/connections.hooks'

interface ConnectionRowProps {
  state: ConnectionState
  isPending: boolean
  errorMessage: string | null
  onConnect: () => void
  onDisconnect: () => void
}

/**
 * One row per supported harness in the editorial MCP install board.
 * Hairline-separated (parent applies `border-t`), no card frame, no
 * icon square. The whole row is a single click target: clicking
 * anywhere on the row fires the currently visible action.
 *
 *   Not connected   click row -> connect. Row shows `connect →` as
 *                    the visual label (mono uppercase accent orange).
 *   Connected       click row -> disconnect. Row shows
 *                    `● connected · disconnect →` as the label.
 *
 * The row highlights on hover / focus / active with `bg-card-tint`
 * so the affordance is unambiguous. Errors render as a red hairline
 * strip below the row (still inside the button so hovering the whole
 * thing keeps the highlight).
 */
export function ConnectionRow({
  state,
  isPending,
  errorMessage,
  onConnect,
  onDisconnect,
}: ConnectionRowProps) {
  return (
    <button
      type="button"
      onClick={state.installed ? onDisconnect : onConnect}
      disabled={isPending}
      aria-label={
        state.installed
          ? `Disconnect ${state.harness}`
          : `Connect ${state.harness}`
      }
      className={cn(
        'group block w-full border-border-2 border-t text-left transition-colors',
        'hover:bg-card-tint focus-visible:bg-card-tint focus-visible:outline-none',
        'disabled:cursor-not-allowed disabled:opacity-70',
      )}
    >
      <div className="flex items-center gap-3 px-2 py-3">
        <HarnessIcon harness={state.harness} className="size-5 shrink-0" />
        <div className="min-w-0 flex-1">
          <div className="font-semibold text-[14px] text-ink-1">
            {state.harness}
          </div>
          {state.installed && state.configPath && (
            <div className="truncate font-mono text-[11px] text-ink-3">
              {state.configPath}
            </div>
          )}
        </div>
        <RowAction state={state} isPending={isPending} />
      </div>
      {errorMessage && (
        <div className="px-10 pb-2 font-mono text-[11.5px] text-red-600">
          {errorMessage}
        </div>
      )}
    </button>
  )
}

function RowAction({
  state,
  isPending,
}: {
  state: ConnectionState
  isPending: boolean
}) {
  if (isPending) {
    return <Loader2 className="size-3.5 shrink-0 animate-spin text-ink-3" />
  }
  if (state.installed) {
    return (
      <div className="flex shrink-0 items-center gap-3">
        <span className="inline-flex items-center gap-1.5 font-mono text-[11px] text-ink-2 uppercase tracking-[0.08em]">
          <span
            aria-hidden
            className="inline-block size-1.5 rounded-full bg-green"
          />
          connected
        </span>
        <span aria-hidden className="text-ink-4">
          ·
        </span>
        <span className="inline-flex items-center gap-1 font-mono text-[11px] text-ink-3 uppercase tracking-[0.08em] transition-colors group-hover:text-ink-1">
          disconnect
          <span
            aria-hidden
            className="transition-transform group-hover:translate-x-0.5"
          >
            →
          </span>
        </span>
      </div>
    )
  }
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center gap-1.5 font-mono text-[11px] text-accent uppercase tracking-[0.08em]',
        'transition-colors group-hover:text-accent-2',
      )}
    >
      connect
      <span
        aria-hidden
        className="transition-transform group-hover:translate-x-0.5"
      >
        →
      </span>
    </span>
  )
}
