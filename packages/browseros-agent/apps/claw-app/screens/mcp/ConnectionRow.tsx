import { CircleCheck, FileText, Unplug } from 'lucide-react'
import type { ReactNode } from 'react'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import type { Harness } from '@/components/harness/harness.types'
import { StatefulButton } from '@/components/motion/button/stateful'
import { OverflowActions } from '@/components/motion/overflow-actions'
import { cn } from '@/lib/utils'
import type { ConnectionState } from '@/modules/api/connections.hooks'
import { toast } from '@/modules/toast/toast'

interface ConnectionRowProps {
  state: ConnectionState
  isPending: boolean
  errorMessage: string | null
  onConnect: () => void
  onDisconnect: () => void
}

/** One AI agent: whether it can use neo, and the one thing to do about it. */
export function ConnectionRow({
  state,
  isPending,
  errorMessage,
  onConnect,
  onDisconnect,
}: ConnectionRowProps) {
  const name = state.harness as Harness

  return (
    <AgentRow
      logo={<HarnessIcon harness={name} className="size-[22px]" />}
      name={name}
      sub={state.installed ? 'Ready to use' : 'Not connected'}
      error={errorMessage}
      action={
        state.installed ? (
          <span className="flex items-center gap-2.5">
            <span className="inline-flex items-center gap-1.5 font-semibold text-[14px] text-green-ink">
              <CircleCheck aria-hidden className="size-4" />
              Connected
            </span>
            <OverflowActions
              size="sm"
              openLabel={`More for ${name}`}
              closeLabel="Close"
              primaryActions={[]}
              overflowActions={[
                ...(state.configPath
                  ? [
                      {
                        id: 'settings-file',
                        label: 'Settings file',
                        ariaLabel: `Show ${name}’s settings file`,
                        icon: <FileText className="size-4" />,
                        onClick: () =>
                          toast({
                            title: `${name} keeps its neo settings here`,
                            description: state.configPath,
                          }),
                      },
                    ]
                  : []),
                {
                  id: 'disconnect',
                  label: 'Disconnect',
                  ariaLabel: `Disconnect ${name}`,
                  icon: <Unplug className="size-4" />,
                  disabled: isPending,
                  onClick: onDisconnect,
                },
              ]}
              collapseOnAction
              classNames={{
                track: 'border-border-strong',
                toggle: 'bg-transparent text-ink-2 hover:bg-bg-sunken',
                action: 'font-semibold text-ink-2',
              }}
            />
          </span>
        ) : (
          <StatefulButton
            variant="secondary"
            size="sm"
            state={isPending ? 'loading' : 'idle'}
            loadingText="Connecting"
            disabled={isPending}
            onClick={onConnect}
            aria-label={`Connect ${name}`}
            className="font-semibold"
          >
            Connect
          </StatefulButton>
        )
      }
    />
  )
}

/** The row shape every agent on this page shares, the Claude desktop app included. */
export function AgentRow({
  logo,
  name,
  sub,
  action,
  error,
  className,
}: {
  logo: ReactNode
  name: string
  sub: string
  action: ReactNode
  error?: string | null
  className?: string
}) {
  return (
    <li
      className={cn(
        'grid grid-cols-[44px_minmax(0,1fr)] items-center gap-x-4 gap-y-2.5 px-[18px] py-4 sm:grid-cols-[44px_minmax(0,1fr)_auto]',
        className,
      )}
    >
      <span
        aria-hidden
        className="grid size-11 place-items-center rounded-xl border border-border-2 bg-card"
      >
        {logo}
      </span>
      <span className="flex min-w-0 flex-col">
        <span className="font-semibold text-[15.5px] text-cyanotype-ink">
          {name}
        </span>
        <span className="text-[13.5px] text-ink-3">{sub}</span>
      </span>
      <span className="col-start-2 justify-self-start sm:col-start-auto sm:justify-self-end">
        {action}
      </span>
      {error ? (
        <p
          role="alert"
          className="col-span-full text-[13.5px] text-red-ink sm:col-start-2"
        >
          {error}
        </p>
      ) : null}
    </li>
  )
}
