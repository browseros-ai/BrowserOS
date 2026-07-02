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
 * icon square. State voices, all in mono uppercase:
 *
 *   Not connected   `connect →` (accent-orange link, arrow slides
 *                    right on hover)
 *   Connected       `● connected · disconnect →` (small green dot,
 *                    mono ink-2 label, ink-3 action link)
 *
 * Errors render below the row as a red hairline strip. The BrowserOS-
 * internal `Built-in` variant no longer exists on this screen; the
 * parent filters those harnesses out of the render list.
 */
export function ConnectionRow({
  state,
  isPending,
  errorMessage,
  onConnect,
  onDisconnect,
}: ConnectionRowProps) {
  return (
    <div className="border-border-2 border-t">
      <div className="flex items-center gap-3 py-3">
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
        {state.installed ? (
          <ConnectedAction onDisconnect={onDisconnect} isPending={isPending} />
        ) : (
          <ConnectAction onConnect={onConnect} isPending={isPending} />
        )}
      </div>
      {errorMessage && (
        <div className="py-2 pl-8 font-mono text-[11.5px] text-red-600">
          {errorMessage}
        </div>
      )}
    </div>
  )
}

function ConnectAction({
  onConnect,
  isPending,
}: {
  onConnect: () => void
  isPending: boolean
}) {
  return (
    <button
      type="button"
      onClick={onConnect}
      disabled={isPending}
      className={cn(
        'group inline-flex shrink-0 items-center gap-1.5 font-mono text-[11px] text-accent uppercase tracking-[0.08em] transition-colors hover:text-accent-2',
        'disabled:cursor-not-allowed disabled:opacity-60',
      )}
    >
      {isPending ? (
        <Loader2 className="size-3.5 animate-spin" />
      ) : (
        <>
          connect
          <span
            aria-hidden
            className="transition-transform group-hover:translate-x-0.5"
          >
            →
          </span>
        </>
      )}
    </button>
  )
}

function ConnectedAction({
  onDisconnect,
  isPending,
}: {
  onDisconnect: () => void
  isPending: boolean
}) {
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
      <button
        type="button"
        onClick={onDisconnect}
        disabled={isPending}
        className="group inline-flex items-center gap-1 font-mono text-[11px] text-ink-3 uppercase tracking-[0.08em] transition-colors hover:text-ink-1 disabled:cursor-not-allowed disabled:opacity-60"
      >
        {isPending ? (
          <Loader2 className="size-3.5 animate-spin" />
        ) : (
          <>
            disconnect
            <span
              aria-hidden
              className="transition-transform group-hover:translate-x-0.5"
            >
              →
            </span>
          </>
        )}
      </button>
    </div>
  )
}
