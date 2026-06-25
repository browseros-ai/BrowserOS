import { ChevronLeft, Copy, ExternalLink, Settings2 } from 'lucide-react'
import { useState } from 'react'
import { NavLink } from 'react-router'
import { Button } from '@/components/ui/button'
import type { TaskDetail } from '@/modules/api/audit.hooks'
import { formatDuration } from '@/screens/audit/audit.helpers'
import { AgentDot } from './AgentDot'
import { StatusBadge } from './StatusBadge'

interface TaskHeaderProps {
  task: TaskDetail
}

export function TaskHeader({ task }: TaskHeaderProps) {
  const [copied, setCopied] = useState(false)
  const finalUrl = lastUrl(task) ?? task.dispatches[0]?.url ?? null

  return (
    <section className="space-y-4">
      <NavLink
        to="/audit"
        className="inline-flex items-center gap-1 text-[12.5px] text-ink-3 hover:text-ink-1"
      >
        <ChevronLeft className="size-3.5" />
        Back to Audit
      </NavLink>

      <header className="rounded-2xl border border-border-2 bg-card p-5">
        <div className="flex items-start justify-between gap-4">
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <AgentDot slug={task.slug} />
              <span className="font-semibold text-ink-1">
                {task.agentLabel}
              </span>
              <StatusBadge status={task.status} />
              {task.errorCount > 0 && (
                <span className="text-[12.5px] text-red-600 dark:text-red-400">
                  {task.errorCount} error{task.errorCount === 1 ? '' : 's'}
                </span>
              )}
            </div>
            <h1 className="font-extrabold text-2xl tracking-tight">
              {task.title}
            </h1>
          </div>
        </div>

        <dl className="mt-5 grid grid-cols-2 gap-x-6 gap-y-2 text-[12.5px] md:grid-cols-4">
          <div>
            <dt className="text-ink-3">Started</dt>
            <dd className="font-mono text-ink-2">
              {new Date(task.startedAt).toLocaleString()}
            </dd>
          </div>
          <div>
            <dt className="text-ink-3">Ended</dt>
            <dd className="font-mono text-ink-2">
              {task.endedAt
                ? new Date(task.endedAt).toLocaleString()
                : task.status === 'live'
                  ? 'still running'
                  : 'idle'}
            </dd>
          </div>
          <div>
            <dt className="text-ink-3">Duration</dt>
            <dd className="font-mono text-ink-2">
              {formatDuration(task.durationMs)}
            </dd>
          </div>
          <div>
            <dt className="text-ink-3">Tools</dt>
            <dd className="font-mono text-ink-2">{task.dispatchCount}</dd>
          </div>
          <div className="col-span-2">
            <dt className="text-ink-3">Site</dt>
            <dd className="font-mono text-ink-2">{task.site ?? 'none'}</dd>
          </div>
          <div className="col-span-2">
            <dt className="text-ink-3">Session</dt>
            <dd className="flex items-center gap-2 font-mono text-ink-2">
              <span className="truncate">{task.sessionId}</span>
              <button
                type="button"
                onClick={() => {
                  void navigator.clipboard
                    .writeText(task.sessionId)
                    .then(() => {
                      setCopied(true)
                      setTimeout(() => setCopied(false), 1500)
                    })
                }}
                className="rounded p-1 text-ink-3 hover:bg-bg-sunken"
                aria-label="Copy session id"
              >
                <Copy className="size-3" />
              </button>
              {copied && (
                <span className="text-[11px] text-accent">copied</span>
              )}
            </dd>
          </div>
        </dl>

        <div className="mt-5 flex flex-wrap gap-2">
          {finalUrl && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => window.open(finalUrl, '_blank', 'noreferrer')}
            >
              <ExternalLink className="mr-1.5 size-3.5" />
              Open final URL
            </Button>
          )}
          {task.site && (
            <Button variant="ghost" size="sm">
              <Settings2 className="mr-1.5 size-3.5" />
              Make a rule on {task.site}
            </Button>
          )}
        </div>
      </header>
    </section>
  )
}

function lastUrl(task: TaskDetail): string | null {
  for (let i = task.dispatches.length - 1; i >= 0; i--) {
    const url = task.dispatches[i]?.url
    if (url) return url
  }
  return null
}
