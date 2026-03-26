import dayjs from 'dayjs'
import duration from 'dayjs/plugin/duration'
import {
  AlertCircle,
  Check,
  CheckCircle2,
  Copy,
  Loader2,
  RotateCcw,
  Square,
  XCircle,
} from 'lucide-react'
import { type FC, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { ScrollArea } from '@/components/ui/scroll-area'
import type { ScheduledJobRun } from '@/lib/schedules/scheduleTypes'
import { MessageResponse } from './message'

dayjs.extend(duration)

interface RunResultDialogProps {
  run: ScheduledJobRun | null
  jobName?: string
  onOpenChange: (open: boolean) => void
  onCancelRun?: (runId: string) => void
  onRetryRun?: (jobId: string) => void
}

const formatDateTime = (dateStr: string) =>
  dayjs(dateStr).format('MMM D, YYYY, h:mm A')

function formatDuration(startedAt: string, completedAt?: string): string {
  if (!completedAt) return 'Still running'
  const diff = dayjs(completedAt).diff(dayjs(startedAt))
  const d = dayjs.duration(diff)
  const mins = Math.floor(d.asMinutes())
  const secs = d.seconds()
  if (mins === 0) return `${secs} seconds`
  return `${mins}m ${secs}s`
}

export const RunResultDialog: FC<RunResultDialogProps> = ({
  run,
  jobName,
  onOpenChange,
  onCancelRun,
  onRetryRun,
}) => {
  const [copied, setCopied] = useState(false)

  const handleCopy = async () => {
    if (!run?.result) return
    await navigator.clipboard.writeText(run.result)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  if (!run) return null

  return (
    <Dialog open={!!run} onOpenChange={onOpenChange}>
      <DialogContent className="sm:w-[70vw] sm:max-w-4xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {run.status === 'completed' ? (
              <CheckCircle2 className="h-5 w-5 text-green-500" />
            ) : run.status === 'failed' ? (
              <XCircle className="h-5 w-5 text-destructive" />
            ) : (
              <Loader2 className="h-5 w-5 animate-spin text-accent-orange" />
            )}
            {jobName || 'Run Result'}
          </DialogTitle>
          <div className="text-muted-foreground text-sm">
            {formatDateTime(run.startedAt)} •{' '}
            {formatDuration(run.startedAt, run.completedAt)}
          </div>
        </DialogHeader>

        <ScrollArea className="max-h-[70vh]">
          {run.status === 'failed' && run.result ? (
            <div className="flex flex-col gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4">
              <div className="flex items-center gap-2 text-muted-foreground">
                <AlertCircle className="h-5 w-5" />
                <span className="font-medium text-sm">Task failed</span>
              </div>
              <p className="text-destructive text-sm">{run.result}</p>
            </div>
          ) : run.result ? (
            <div className="prose prose-sm dark:prose-invert [&_[data-streamdown='code-block']]:!w-full [&_[data-streamdown='table-wrapper']]:!w-full max-w-none break-words rounded-lg border border-border bg-muted/50 p-4 [&_[data-streamdown='table-wrapper']]:overflow-x-auto">
              <MessageResponse>{run.result}</MessageResponse>
            </div>
          ) : (
            <div className="rounded-lg border border-border bg-muted/50 p-4 text-muted-foreground text-sm">
              No result available
            </div>
          )}
        </ScrollArea>

        <DialogFooter>
          {run.status === 'running' && onCancelRun && (
            <Button variant="destructive" onClick={() => onCancelRun(run.id)}>
              <Square className="h-4 w-4" />
              Cancel
            </Button>
          )}
          {run.status === 'failed' && onRetryRun && (
            <Button
              variant="outline"
              onClick={() => {
                onRetryRun(run.jobId)
                onOpenChange(false)
              }}
            >
              <RotateCcw className="h-4 w-4" />
              Retry
            </Button>
          )}
          {run.result && (
            <Button
              variant="outline"
              onClick={handleCopy}
              className="mr-2 sm:mr-0"
            >
              {copied ? (
                <>
                  <Check className="h-4 w-4" />
                  Copied
                </>
              ) : (
                <>
                  <Copy className="h-4 w-4" />
                  Copy
                </>
              )}
            </Button>
          )}
          <Button onClick={() => onOpenChange(false)}>Close</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
