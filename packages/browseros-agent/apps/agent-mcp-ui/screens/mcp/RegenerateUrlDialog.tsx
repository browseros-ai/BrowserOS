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
import { buttonVariants } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { AgentProfile } from '@/modules/api/agents.hooks'

interface RegenerateUrlDialogProps {
  profile: AgentProfile | null
  isRegenerating: boolean
  onConfirm: (profile: AgentProfile) => void
  onCancel: () => void
}

export function RegenerateUrlDialog({
  profile,
  isRegenerating,
  onConfirm,
  onCancel,
}: RegenerateUrlDialogProps) {
  return (
    <AlertDialog
      open={profile !== null}
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Rotate this MCP URL?</AlertDialogTitle>
          <AlertDialogDescription>
            The current URL stops working the moment the new one is issued. The
            old entry in {profile?.harness ?? 'your harness'} is removed
            automatically and the new URL is installed in its place, so the
            harness keeps a working connector continuously. Any place where you
            pasted the old URL by hand (CI configs, scripts) will need to be
            updated.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={isRegenerating}>
            Cancel
          </AlertDialogCancel>
          <AlertDialogAction
            disabled={isRegenerating || profile === null}
            onClick={(event) => {
              event.preventDefault()
              if (profile) onConfirm(profile)
            }}
            className={cn(buttonVariants({ variant: 'destructive' }))}
          >
            {isRegenerating ? 'Rotating…' : 'Rotate URL'}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
