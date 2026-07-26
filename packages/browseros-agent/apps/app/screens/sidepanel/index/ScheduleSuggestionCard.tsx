import { Clock, X } from 'lucide-react'
import { type FC, useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  BREADCRUMB_SCHEDULE_CLICKED_EVENT,
  BREADCRUMB_SCHEDULE_DISMISSED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import type { NudgeData } from './getMessageSegments'

export interface ScheduleSuggestionCardProps {
  data: NudgeData
  isLastMessage: boolean
}

export const ScheduleSuggestionCard: FC<ScheduleSuggestionCardProps> = ({
  data,
  isLastMessage,
}) => {
  const [dismissed, setDismissed] = useState(!isLastMessage)

  const suggestedName = (data.suggestedName as string) ?? 'Scheduled Task'
  const scheduleType = (data.scheduleType as string) ?? 'daily'
  const scheduleTime = (data.scheduleTime as string) ?? '09:00'
  const query = (data.query as string) ?? ''

  useEffect(() => {
    if (!isLastMessage) {
      setDismissed(true)
    }
  }, [isLastMessage])

  const handleDismiss = () => {
    track(BREADCRUMB_SCHEDULE_DISMISSED_EVENT, {
      suggested_name: suggestedName,
    })
    setDismissed(true)
  }

  if (dismissed) return null

  const scheduleLabel =
    scheduleType === 'daily' ? `daily at ${scheduleTime}` : 'every hour'

  const handleSchedule = () => {
    track(BREADCRUMB_SCHEDULE_CLICKED_EVENT, {
      suggested_name: suggestedName,
      schedule_type: scheduleType,
    })

    const params = new URLSearchParams({
      name: suggestedName,
      query,
      scheduleType,
      scheduleTime,
      openDialog: 'true',
    })

    const url = chrome.runtime.getURL(
      `app.html#/scheduled?${params.toString()}`,
    )
    chrome.tabs.create({ url })
  }

  return (
    <div className="relative rounded-lg border border-border/50 bg-card p-4 shadow-sm">
      <button
        type="button"
        onClick={handleDismiss}
        className="absolute top-2 right-2 rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
      >
        <X className="h-4 w-4" />
      </button>

      <div className="flex items-start gap-3 pr-6">
        <Clock className="h-5 w-5 shrink-0 text-[var(--accent-orange)]" />
        <div>
          <p className="font-medium text-sm">Run this automatically?</p>
          <p className="mt-1 text-muted-foreground text-xs">
            &ldquo;{suggestedName}&rdquo; &mdash; I can run this {scheduleLabel}
          </p>
        </div>
      </div>

      <div className="mt-3 flex gap-2">
        <Button size="sm" onClick={handleSchedule}>
          Schedule this task
        </Button>
        <Button size="sm" variant="ghost" onClick={handleDismiss}>
          Maybe later
        </Button>
      </div>
    </div>
  )
}
