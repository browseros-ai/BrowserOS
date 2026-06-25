import {
  Check,
  Eye,
  Layers,
  Loader2,
  Pause,
  Play,
  StopCircle,
  User,
  X,
} from 'lucide-react'
import { StatusBadge } from '@/components/cockpit/StatusBadge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type {
  ActionLogItem,
  ResolvedLogItem,
  RunDetail,
  RunStats,
} from '@/modules/api/run.hooks'
import { ApprovalCard } from './ApprovalCard'
import { BlockNotice } from './BlockNotice'
import { HandoffNotice } from './HandoffNotice'
import { VERB_META } from './live-run.helpers'

interface ActivityPanelProps {
  run: RunDetail
  paused: boolean
  onAllowOnce: () => void
  onAllowAlways: () => void
  onBlock: () => void
  onContinue: () => void
  onPause: () => void
  onStop: () => void
  onManageBlock: () => void
}

/**
 * Right-side docked panel showing the browser-action log and any
 * pinned approval/handoff/block overlays. Mirrors the prototype's
 * AssistantPanel but uses shadcn primitives and lucide icons.
 */
export function ActivityPanel({
  run,
  paused,
  onAllowOnce,
  onAllowAlways,
  onBlock,
  onContinue,
  onPause,
  onStop,
  onManageBlock,
}: ActivityPanelProps) {
  const events = run.log
  const isActive =
    run.status === 'running' ||
    run.status === 'live' ||
    !!run.pending ||
    !!run.blocked ||
    !!run.handoff
  return (
    <aside className="flex w-[380px] shrink-0 flex-col border-border border-l bg-card">
      <header className="flex h-12 items-center gap-2 border-border border-b px-4">
        <span className="flex size-5 items-center justify-center rounded-md bg-accent-tint text-accent">
          <Eye className="size-3.5" />
        </span>
        <span className="font-bold text-ink text-sm">Agent activity</span>
      </header>

      <div className="relative border-border border-b px-4 py-3">
        <span
          aria-hidden
          className="absolute top-3 bottom-3 left-0 w-[3px] rounded-r bg-accent"
        />
        <div className="mb-1 flex items-center gap-2 pl-4">
          <span className="size-2 shrink-0 rounded-full bg-accent" />
          <span className="min-w-0 flex-1 truncate font-bold text-ink text-sm">
            {run.agentLabel}
          </span>
          <StatusBadge status={run.status} />
        </div>
        <p className="pl-4 text-ink-2 text-xs leading-snug">{run.task}</p>
        <div className="mt-2 flex items-center gap-1.5 pl-4 text-ink-3 text-xs">
          <Layers className="size-3" />1 tab group · driven by{' '}
          <strong className="font-semibold text-ink-2">{run.harness}</strong>
        </div>
      </div>

      {run.blocked && (
        <div className="flex items-center gap-3 border-border border-b bg-red-tint px-4 py-2.5">
          <span className="flex size-7 shrink-0 items-center justify-center rounded-lg border border-[#EBC4BF] bg-card text-red">
            <X className="size-3.5" />
          </span>
          <div className="min-w-0 flex-1">
            <div className="font-bold text-[10.5px] text-red uppercase tracking-wider">
              1 action blocked
            </div>
            <div className="truncate font-semibold text-ink text-xs">
              {run.blocked.detail}
            </div>
          </div>
        </div>
      )}

      <div className="flex flex-1 flex-col gap-3 overflow-y-auto px-4 py-3">
        <div className="flex items-center gap-1.5 font-bold text-[10.5px] text-ink-4 uppercase tracking-wider">
          <Eye className="size-3" />
          Browser actions
        </div>
        {events.length === 0 &&
          !run.current &&
          !run.pending &&
          !run.blocked && (
            <p className="text-ink-3 text-xs">
              Waiting for the agent's first action…
            </p>
          )}
        {events.map((item) =>
          item.kind === 'resolved' ? (
            <ResolvedTag key={item.id} item={item} />
          ) : item.kind === 'done' ? (
            <p key={item.id} className="text-ink-2 text-xs leading-snug">
              {item.text}
            </p>
          ) : (
            <ActionRow key={item.id} item={item} />
          ),
        )}
        {run.current && <ActionRow item={run.current} running />}
        {run.blocked && (
          <BlockNotice block={run.blocked} onManage={onManageBlock} />
        )}
        {run.handoff && (
          <HandoffNotice
            handoff={run.handoff}
            onContinue={onContinue}
            onCancel={onStop}
          />
        )}
        {run.pending && (
          <ApprovalCard
            approval={run.pending}
            harnessLabel={run.harness}
            onAllowOnce={onAllowOnce}
            onAllowAlways={onAllowAlways}
            onBlock={onBlock}
          />
        )}
      </div>

      <StatStrip elapsed={run.elapsed} stats={run.stats} />

      {isActive && !run.handoff && (
        <div className="flex gap-1.5 px-4 py-3">
          <Button
            type="button"
            variant="outline"
            onClick={onPause}
            className="flex-1"
            size="sm"
          >
            {paused ? (
              <Play className="size-3.5" />
            ) : (
              <Pause className="size-3.5" />
            )}
            {paused ? 'Resume' : 'Pause'}
          </Button>
          <Button
            type="button"
            variant="outline"
            onClick={onStop}
            className="flex-1"
            size="sm"
          >
            <StopCircle className="size-3.5" />
            Stop
          </Button>
        </div>
      )}
    </aside>
  )
}

/* ---------------------------------------------------------------------------
 * Sub-components, kept private to the activity panel.
 * -------------------------------------------------------------------------*/

function ActionRow({
  item,
  running = false,
}: {
  item: ActionLogItem
  running?: boolean
}) {
  const meta = VERB_META[item.verb]
  const Icon = running ? Loader2 : meta.Icon
  return (
    <div className="flex gap-2.5">
      <span
        className={cn(
          'mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-md',
          running ? 'bg-accent-tint text-accent' : 'bg-bg-sunken',
          !running && meta.iconClass,
        )}
      >
        <Icon className={cn('size-3.5', running && 'animate-spin')} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-ink text-sm leading-snug">
          <span className="font-semibold">{meta.label}</span>{' '}
          <code className="rounded-md bg-bg-sunken px-1.5 py-0.5 font-mono text-[11.5px] text-ink-2">
            {item.node}
          </code>
        </div>
        {item.text && (
          <p className="mt-1 text-ink-2 text-xs leading-snug">{item.text}</p>
        )}
        {running && (
          <p className="mt-1 font-semibold text-[11.5px] text-accent-ink">
            working…
          </p>
        )}
      </div>
    </div>
  )
}

function ResolvedTag({ item }: { item: ResolvedLogItem }) {
  const allowed = item.res !== 'blocked'
  const SignIcon = item.res === 'handed-back' ? User : allowed ? Check : X
  return (
    <div className="flex items-center gap-2.5">
      <span
        className={cn(
          'flex size-6 shrink-0 items-center justify-center rounded-md',
          allowed ? 'bg-green-tint text-green' : 'bg-red-tint text-red',
        )}
      >
        <SignIcon className="size-3.5" />
      </span>
      <p className="text-ink-2 text-xs">
        {item.res === 'once' && (
          <>
            You <strong className="font-semibold text-ink">allowed once</strong>{' '}
            · {item.action} on {item.domain}
          </>
        )}
        {item.res === 'always' && (
          <>
            You <strong className="font-semibold text-ink">always allow</strong>{' '}
            {item.action} on {item.domain}
          </>
        )}
        {item.res === 'blocked' && (
          <>
            You <strong className="font-semibold text-red">blocked</strong>{' '}
            {item.action} on {item.domain}
          </>
        )}
        {item.res === 'handed-back' && (
          <>
            You{' '}
            <strong className="font-semibold text-ink">
              finished the {item.action}
            </strong>{' '}
            · handed back to the agent
          </>
        )}
      </p>
    </div>
  )
}

function StatStrip({ elapsed, stats }: { elapsed: string; stats: RunStats }) {
  const cells = [
    { label: 'Elapsed', value: elapsed },
    { label: 'Tokens', value: stats.tokens, ref: stats.tokensRef },
    { label: 'Steps', value: stats.steps },
  ]
  return (
    <div className="flex border-border border-t border-b bg-card-tint">
      {cells.map((cell, i) => (
        <div
          key={cell.label}
          className={cn(
            'flex-1 px-3 py-2',
            i < cells.length - 1 && 'border-border border-r',
          )}
        >
          <div className="font-bold text-[10px] text-ink-4 uppercase tracking-wider">
            {cell.label}
          </div>
          <div className="mt-0.5 flex items-baseline gap-1.5 font-bold font-mono text-ink text-sm">
            {cell.value}
            {cell.ref && (
              <span className="font-medium text-[11px] text-ink-4 line-through">
                {cell.ref}
              </span>
            )}
          </div>
        </div>
      ))}
    </div>
  )
}
