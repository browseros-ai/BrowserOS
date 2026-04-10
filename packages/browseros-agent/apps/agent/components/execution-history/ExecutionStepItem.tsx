import {
  CheckCircle2,
  ChevronDown,
  CircleDotDashed,
  Clock3,
  ShieldAlert,
  ShieldCheck,
  XCircle,
} from 'lucide-react'
import { type FC, useState } from 'react'
import { ToolInput, ToolOutput } from '@/components/ai-elements/tool'
import { Badge } from '@/components/ui/badge'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import type { ExecutionStepRecord } from '@/lib/execution-history/types'
import { cn } from '@/lib/utils'

const formatToolName = (name: string) =>
  name
    .replace(/_/g, ' ')
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .replace(/^./, (value) => value.toUpperCase())

const formatStateLabel = (state: ExecutionStepRecord['state']) => {
  if (state === 'input-streaming') return 'Preparing'
  if (state === 'input-available') return 'Running'
  if (state === 'approval-requested') return 'Approval Needed'
  if (state === 'approval-responded') return 'Approval Responded'
  if (state === 'output-available') return 'Completed'
  if (state === 'output-denied') return 'Denied'
  return 'Error'
}

const getStateIcon = (step: ExecutionStepRecord) => {
  if (step.state === 'output-available') {
    return <CheckCircle2 className="h-4 w-4 text-green-500" />
  }

  if (
    step.state === 'input-streaming' ||
    step.state === 'input-available' ||
    step.state === 'approval-requested'
  ) {
    return <Clock3 className="h-4 w-4 text-[var(--accent-orange)]" />
  }

  if (step.state === 'approval-responded') {
    return <ShieldCheck className="h-4 w-4 text-blue-500" />
  }

  if (step.state === 'output-denied') {
    return <ShieldAlert className="h-4 w-4 text-orange-500" />
  }

  if (step.state === 'output-error') {
    return <XCircle className="h-4 w-4 text-destructive" />
  }

  return <CircleDotDashed className="h-4 w-4 text-muted-foreground" />
}

const isAclBlocked = (step: ExecutionStepRecord) =>
  Boolean(
    step.errorText?.includes('Action blocked by ACL rule') ||
      step.approval?.reason?.includes('Action blocked by ACL rule') ||
      step.previewText === 'Blocked by ACL rule',
  )

const shouldShowPreview = (step: ExecutionStepRecord) =>
  step.state === 'input-streaming' ||
  step.state === 'input-available' ||
  step.state === 'approval-requested' ||
  step.state === 'approval-responded'

export const ExecutionStepItem: FC<{
  step: ExecutionStepRecord
  defaultOpen?: boolean
}> = ({ step, defaultOpen = false }) => {
  const [open, setOpen] = useState(defaultOpen)
  const deniedReason =
    step.state === 'output-denied' ? step.approval?.reason : undefined

  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <div className="rounded-xl border border-border/60 bg-card/60">
        <CollapsibleTrigger asChild>
          <button
            type="button"
            className="flex w-full items-start gap-3 px-4 py-3 text-left"
          >
            <div className="mt-0.5 shrink-0">{getStateIcon(step)}</div>
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2">
                <p className="font-medium text-foreground text-sm">
                  {formatToolName(step.toolName)}
                </p>
                <Badge variant="secondary">
                  {formatStateLabel(step.state)}
                </Badge>
                {isAclBlocked(step) && (
                  <Badge variant="outline">ACL Blocked</Badge>
                )}
              </div>
              {shouldShowPreview(step) && (
                <p className="mt-1 text-muted-foreground text-xs">
                  {step.previewText}
                </p>
              )}
            </div>
            <ChevronDown
              className={cn(
                'mt-0.5 h-4 w-4 shrink-0 text-muted-foreground transition-transform',
                open && 'rotate-180',
              )}
            />
          </button>
        </CollapsibleTrigger>
        <CollapsibleContent className="border-border/60 border-t">
          {step.input !== undefined && <ToolInput input={step.input} />}
          {step.state === 'output-denied' ? (
            <div className="space-y-2 p-4">
              <h4 className="font-medium text-muted-foreground text-xs uppercase tracking-wide">
                Result
              </h4>
              <div className="rounded-md bg-orange-500/10 p-3 text-orange-700 text-sm dark:text-orange-300">
                {deniedReason ?? 'The requested action was denied.'}
              </div>
            </div>
          ) : (
            <ToolOutput
              output={step.output}
              errorText={step.errorText}
              className="pt-0"
            />
          )}
        </CollapsibleContent>
      </div>
    </Collapsible>
  )
}
