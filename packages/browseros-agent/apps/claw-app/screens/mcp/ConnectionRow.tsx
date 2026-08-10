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
 *   Not connected   click row -> connect. Row shows `Connect →` as
 *                    the visual label, in cobalt.
 *   Connected       click row -> disconnect. Row shows
 *                    `Connected · Disconnect →` as the label. The green
 *                    word carries the state, so there is no status dot.
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
        'group block w-full border-cyanotype-border border-t text-left transition-colors',
        'hover:bg-card-tint focus-visible:bg-card-tint focus-visible:outline-none',
        'disabled:cursor-not-allowed disabled:opacity-70',
      )}
    >
      <div className="flex items-center gap-3 px-2 py-3">
        <HarnessIcon harness={state.harness} className="size-5 shrink-0" />
        <div className="min-w-0 flex-1">
          <div className="font-semibold text-[14px] text-cyanotype-ink">
            {state.harness}
          </div>
          {state.installed && state.configPath && (
            <div className="truncate font-mono text-[12px] text-cyanotype-muted">
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
        <span className="font-semibold text-[12px] text-cyanotype-live-text">
          Connected
        </span>
        <span aria-hidden className="text-[16px] text-cyanotype-border">
          ·
        </span>
        <span className="inline-flex items-center gap-1 text-[12px] text-cyanotype-muted transition-colors group-hover:text-cyanotype-ink">
          Disconnect
          <ActionArrow />
        </span>
      </div>
    )
  }
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center gap-1.5 text-[12px] text-cyanotype-blue',
        'transition-colors group-hover:text-cyanotype-blue-hover',
      )}
    >
      Connect
      <ActionArrow />
    </span>
  )
}

function ActionArrow() {
  return (
    <span
      aria-hidden
      className="font-mono text-[11px] tracking-[0.08em] transition-transform group-hover:translate-x-0.5"
    >
      →
    </span>
  )
}
