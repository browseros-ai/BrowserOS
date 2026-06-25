import { Check, Loader2 } from 'lucide-react'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import type { ConnectionState } from '@/modules/api/connections.hooks'

interface ConnectionRowProps {
  state: ConnectionState
  isPending: boolean
  errorMessage: string | null
  onConnect: () => void
  onDisconnect: () => void
}

/**
 * One row per supported harness. Click "Connect" to write BrowserOS
 * into the harness's MCP config file; the row flips to a green
 * "Connected" pill on success. Click "Disconnect" to remove it.
 * Errors render below the row in a small red strip.
 *
 * BrowserOS-internal harnesses (Hermes, OpenClaw) ship `installed:
 * true` from the server and render a non-interactive "Built-in" pill.
 */
export function ConnectionRow({
  state,
  isPending,
  errorMessage,
  onConnect,
  onDisconnect,
}: ConnectionRowProps) {
  const internal = state.agentId === null

  return (
    <div className="border-border-2 border-b last:border-b-0">
      <div className="flex items-center gap-3 px-4 py-3">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-card">
          <HarnessIcon harness={state.harness} className="size-5" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="font-bold text-[14px]">{state.harness}</div>
          {state.installed && state.configPath && (
            <div className="truncate font-mono text-[11px] text-ink-3">
              {state.configPath}
            </div>
          )}
          {internal && (
            <div className="text-[11.5px] text-ink-3">
              Runs inside BrowserOS
            </div>
          )}
        </div>
        {internal ? (
          <span className="inline-flex shrink-0 items-center gap-1.5 rounded-full bg-bg-sunken px-3 py-1.5 font-bold text-[12px] text-ink-2">
            <Check className="size-3" />
            Built-in
          </span>
        ) : state.installed ? (
          <div className="flex shrink-0 items-center gap-2">
            <span className="inline-flex items-center gap-1.5 rounded-full bg-green-tint px-3 py-1 font-bold text-[12px] text-green">
              <Check className="size-3" />
              Connected
            </span>
            <button
              type="button"
              onClick={onDisconnect}
              disabled={isPending}
              className="rounded-md px-2 py-1 font-semibold text-[12px] text-ink-3 transition hover:bg-bg-sunken hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
            >
              {isPending ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                'Disconnect'
              )}
            </button>
          </div>
        ) : (
          <button
            type="button"
            onClick={onConnect}
            disabled={isPending}
            className="inline-flex shrink-0 items-center gap-1.5 rounded-md bg-accent px-3.5 py-1.5 font-bold text-[12.5px] text-accent-foreground transition hover:bg-accent-2 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {isPending ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              'Connect'
            )}
          </button>
        )}
      </div>
      {errorMessage && (
        <div className="border-red/10 border-t bg-red-tint px-4 py-2 text-[12px] text-red">
          {errorMessage}
        </div>
      )}
    </div>
  )
}
