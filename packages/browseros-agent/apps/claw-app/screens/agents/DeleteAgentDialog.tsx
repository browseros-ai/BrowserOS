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

interface DeleteAgentDialogProps {
  /** Profile whose row triggered the delete; `null` when the dialog is closed. */
  profile: AgentProfile | null
  isDeleting: boolean
  onConfirm: (profile: AgentProfile) => void
  onCancel: () => void
}

/**
 * Confirm dialog for removing an agent profile. shadcn AlertDialog
 * gives us proper focus trapping and ARIA semantics out of the box,
 * which is why this is not a window.confirm.
 */
export function DeleteAgentDialog({
  profile,
  isDeleting,
  onConfirm,
  onCancel,
}: DeleteAgentDialogProps) {
  return (
    <AlertDialog
      open={profile !== null}
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Remove this agent profile?</AlertDialogTitle>
          <AlertDialogDescription>
            {profile?.name ? `"${profile.name}"` : 'This profile'} stops
            responding to the harness immediately. Its MCP endpoint is
            unregistered and any always-allow grants you gave it are revoked.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={isDeleting}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            disabled={isDeleting || profile === null}
            onClick={(event) => {
              event.preventDefault()
              if (profile) onConfirm(profile)
            }}
            className={cn(buttonVariants({ variant: 'destructive' }))}
          >
            {isDeleting ? 'Removing…' : 'Remove agent'}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
