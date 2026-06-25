import { ArrowLeft, Code, Sparkles, X } from 'lucide-react'
import { useState } from 'react'
import { StatusBadge } from '@/components/cockpit/StatusBadge'
import { Spinner } from '@/components/ui/spinner'
import { cn } from '@/lib/utils'
import { ActivityPanel } from './ActivityPanel'
import { BrowserViewport } from './BrowserViewport'
import { HandoffBanner } from './HandoffBanner'
import { useLiveRunData } from './live-run.data'

export function LiveRun() {
  const { run, runId, isLoading, navigate } = useLiveRunData()
  const [paused, setPaused] = useState(false)
  const [dismissedApproval, setDismissedApproval] = useState(false)
  const [dismissedHandoff, setDismissedHandoff] = useState(false)
  const [dismissedBlock, _setDismissedBlock] = useState(false)

  if (isLoading || !run) {
    return (
      <div className="flex h-full flex-1 items-center justify-center bg-bg-canvas text-ink-3">
        <Spinner />
      </div>
    )
  }

  const close = () => navigate('/')
  const goToBlockSettings = () => navigate('/governance/site-rules')

  const pending = dismissedApproval ? undefined : run.pending
  const handoff = dismissedHandoff ? undefined : run.handoff
  const blocked = dismissedBlock ? undefined : run.blocked

  const HarnessIcon = run.harness === 'Codex' ? Code : Sparkles

  const headerStatus = handoff
    ? 'needs-human'
    : blocked
      ? 'blocked'
      : pending
        ? 'needs-ok'
        : paused
          ? 'stuck'
          : run.status

  return (
    <div className="flex h-screen min-h-0 flex-col bg-bg-canvas">
      <header className="flex h-14 shrink-0 items-center gap-3 border-border border-b bg-card px-5">
        <button
          type="button"
          onClick={close}
          className="flex items-center gap-1.5 font-semibold text-ink-2 text-sm hover:text-ink"
        >
          <ArrowLeft className="size-4" />
          Cockpit
        </button>
        <span className="h-5 w-px bg-border-2" />
        <span
          className={cn(
            'flex size-7 shrink-0 items-center justify-center rounded-lg text-accent',
            run.harness === 'Codex'
              ? 'bg-bg-sunken text-ink-2'
              : 'bg-accent-tint',
          )}
        >
          <HarnessIcon className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate font-extrabold text-ink text-sm tracking-tight">
              {run.agentLabel}
            </span>
            <StatusBadge status={headerStatus} />
          </div>
          <div className="truncate text-ink-3 text-xs">{run.task}</div>
        </div>
        <button
          type="button"
          onClick={close}
          aria-label="Close run"
          className="flex size-8 items-center justify-center rounded-md text-ink-3 hover:bg-bg-sunken hover:text-ink"
        >
          <X className="size-4" />
        </button>
      </header>

      <div className="flex min-h-0 flex-1">
        <BrowserViewport
          site={run.site}
          harness={run.harness}
          paused={paused || !!handoff}
          workingLabel={run.liveLine}
          hideWorkingPill={!!handoff || paused}
          onStop={close}
          overlay={
            handoff && (
              <HandoffBanner
                handoff={handoff}
                onContinue={() => setDismissedHandoff(true)}
                onCancel={close}
              />
            )
          }
        />
        <ActivityPanel
          run={{
            ...run,
            pending,
            handoff,
            blocked,
            status: headerStatus,
          }}
          paused={paused}
          onAllowOnce={() => setDismissedApproval(true)}
          onAllowAlways={() => setDismissedApproval(true)}
          onBlock={() => setDismissedApproval(true)}
          onContinue={() => setDismissedHandoff(true)}
          onPause={() => setPaused((p) => !p)}
          onStop={close}
          onManageBlock={goToBlockSettings}
        />
      </div>

      {!run && (
        <div className="px-5 py-3 text-ink-3 text-xs">
          No run tracked for id {runId}.
        </div>
      )}
    </div>
  )
}
