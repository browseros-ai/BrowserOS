import { useMutation, useQueryClient } from '@tanstack/react-query'
import { Brain, FileText, Loader2, RotateCcw } from 'lucide-react'
import { type FC, type ReactNode, useState } from 'react'
import { toast } from 'sonner'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'
import { MEMORY_QUERY_KEY } from '../memory/useMemoryContent'
import { SOUL_QUERY_KEY } from '../soul/useSoulContent'

type ResetTarget = 'memory' | 'soul'

type ResetAction = {
  target: ResetTarget
  title: string
  description: string
  buttonLabel: string
  icon: ReactNode
}

async function deleteServerResource(
  baseUrl: string,
  resource: ResetTarget,
): Promise<void> {
  const response = await fetch(`${baseUrl}/${resource}`, { method: 'DELETE' })
  if (!response.ok) throw new Error(`HTTP ${response.status}`)
}

export const ResetDataPage: FC = () => {
  const {
    baseUrl,
    isLoading: isUrlLoading,
    error: urlError,
  } = useAgentServerUrl()
  const queryClient = useQueryClient()
  const [pendingAction, setPendingAction] = useState<ResetAction | null>(null)

  const resetMutation = useMutation({
    mutationFn: async (target: ResetTarget) => {
      if (!baseUrl) throw new Error('BrowserOS server URL is unavailable')
      await deleteServerResource(baseUrl, target)
      return target
    },
    onSuccess: async (target) => {
      if (target === 'memory') {
        queryClient.setQueryData([MEMORY_QUERY_KEY, baseUrl], '')
      }
      await queryClient.invalidateQueries({
        queryKey: target === 'memory' ? [MEMORY_QUERY_KEY] : [SOUL_QUERY_KEY],
      })
      toast.success(target === 'memory' ? 'Memory reset' : 'SOUL.md reset')
    },
    onError: (_error, target) => {
      toast.error(
        target === 'memory'
          ? 'Failed to reset memory'
          : 'Failed to reset SOUL.md',
      )
    },
  })

  const actions: ResetAction[] = [
    {
      target: 'memory',
      title: 'Reset memory?',
      description:
        'This deletes CORE.md and daily memory files. This cannot be undone.',
      buttonLabel: 'Reset memory',
      icon: <Brain className="h-4 w-4 text-muted-foreground" />,
    },
    {
      target: 'soul',
      title: 'Reset SOUL.md?',
      description:
        'This replaces SOUL.md with the default template. This cannot be undone.',
      buttonLabel: 'Reset SOUL.md',
      icon: <FileText className="h-4 w-4 text-muted-foreground" />,
    },
  ]

  const isBusy = isUrlLoading || resetMutation.isPending
  const disabled = isBusy || Boolean(urlError) || !baseUrl

  const handleConfirm = () => {
    if (!pendingAction) return
    resetMutation.mutate(pendingAction.target)
    setPendingAction(null)
  }

  return (
    <div className="mx-auto w-full max-w-3xl space-y-6 p-6">
      <div>
        <div className="mb-2 flex items-center gap-2 text-muted-foreground">
          <RotateCcw className="h-4 w-4" />
          <span className="font-medium text-xs uppercase tracking-wider">
            Reset
          </span>
        </div>
        <h1 className="font-semibold text-2xl">Reset Data</h1>
      </div>

      {urlError ? (
        <div className="rounded-lg border border-destructive/50 bg-destructive/5 p-4">
          <p className="text-destructive text-sm">
            BrowserOS server is unavailable.
          </p>
        </div>
      ) : null}

      <div className="space-y-3">
        {actions.map((action) => (
          <div
            key={action.target}
            className="flex flex-col gap-3 rounded-lg border bg-card p-4 shadow-sm sm:flex-row sm:items-center sm:justify-between"
          >
            <div className="flex min-w-0 items-center gap-3">
              {action.icon}
              <div className="min-w-0">
                <h2 className="font-medium text-sm">{action.buttonLabel}</h2>
                <p className="mt-1 text-muted-foreground text-xs">
                  {action.description}
                </p>
              </div>
            </div>
            <Button
              type="button"
              variant="destructive"
              size="sm"
              className="shrink-0"
              disabled={disabled}
              onClick={() => setPendingAction(action)}
            >
              {resetMutation.isPending &&
              resetMutation.variables === action.target ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <RotateCcw className="h-3.5 w-3.5" />
              )}
              {action.buttonLabel}
            </Button>
          </div>
        ))}
      </div>

      <AlertDialog
        open={Boolean(pendingAction)}
        onOpenChange={(open) => {
          if (!open) setPendingAction(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{pendingAction?.title}</AlertDialogTitle>
            <AlertDialogDescription>
              {pendingAction?.description}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleConfirm}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {pendingAction?.buttonLabel}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
