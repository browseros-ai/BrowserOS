import { Check, Clock, Globe, ShieldOff } from 'lucide-react'
import { useState } from 'react'
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
import { Button, buttonVariants } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { cn } from '@/lib/utils'
import type { Grant } from '@/modules/api/grants.hooks'
import { useGrantsData } from './grants.data'

export function GrantsTab() {
  const { grants, isLoading, revokeGrant } = useGrantsData()
  const [pendingRevoke, setPendingRevoke] = useState<Grant | null>(null)

  const onConfirm = (grant: Grant) => {
    revokeGrant.mutate(
      { id: grant.id },
      { onSettled: () => setPendingRevoke(null) },
    )
  }

  return (
    <section className="flex flex-col gap-3">
      <header className="flex items-start justify-between gap-3">
        <p className="max-w-[60ch] text-ink-2 text-sm leading-snug">
          Every approval you marked "Always allow" lives here. Revoke a standing
          grant in one click without hunting through a run timeline.
        </p>
        <span className="shrink-0 rounded-full bg-bg-sunken px-2.5 py-1 font-mono text-ink-3 text-xs">
          {grants.length} grant{grants.length === 1 ? '' : 's'}
        </span>
      </header>

      {isLoading ? (
        <div className="flex justify-center py-12 text-ink-3">
          <Spinner />
        </div>
      ) : grants.length === 0 ? (
        <EmptyState />
      ) : (
        <div className="flex flex-col gap-2">
          {grants.map((grant) => (
            <GrantRow
              key={grant.id}
              grant={grant}
              onRevoke={() => setPendingRevoke(grant)}
            />
          ))}
        </div>
      )}

      <RevokeGrantDialog
        grant={pendingRevoke}
        isRevoking={revokeGrant.isPending}
        onConfirm={onConfirm}
        onCancel={() => setPendingRevoke(null)}
      />
    </section>
  )
}

/* ---------------------------------------------------------------------------
 * Sub-components, kept private to this tab.
 * -------------------------------------------------------------------------*/

interface GrantRowProps {
  grant: Grant
  onRevoke: () => void
}

function GrantRow({ grant, onRevoke }: GrantRowProps) {
  return (
    <div className="flex items-center gap-3 rounded-xl border border-border-2 bg-card p-3.5">
      <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-green-tint text-green">
        <Check className="size-4" />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2 truncate">
          <span className="truncate font-semibold text-ink text-sm">
            {grant.action}
          </span>
          <span className="text-ink-4 text-xs">on</span>
          <span className="inline-flex shrink-0 items-center gap-1 font-mono text-ink-2 text-xs">
            <Globe className="size-3 text-ink-3" />
            {grant.domain}
          </span>
        </div>
        <div className="mt-0.5 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-ink-3 text-xs">
          Granted to <span className="text-ink-2">{grant.grantedTo}</span>
          <span className="text-ink-4">·</span>
          <Clock className="size-3" />
          {grant.when}
          {grant.note && (
            <>
              <span className="text-ink-4">·</span>
              <span className="italic">{grant.note}</span>
            </>
          )}
        </div>
      </div>
      <Button
        type="button"
        variant="outline"
        size="sm"
        onClick={onRevoke}
        className="shrink-0 gap-1.5 px-2.5 text-ink-3 text-xs hover:text-red"
      >
        <ShieldOff className="size-3.5" />
        Revoke
      </Button>
    </div>
  )
}

function EmptyState() {
  return (
    <div className="rounded-xl border border-border border-dashed bg-card py-10 text-center text-ink-3 text-sm">
      No standing grants yet. Approvals marked "Always allow" will land here.
    </div>
  )
}

interface RevokeGrantDialogProps {
  grant: Grant | null
  isRevoking: boolean
  onConfirm: (grant: Grant) => void
  onCancel: () => void
}

function RevokeGrantDialog({
  grant,
  isRevoking,
  onConfirm,
  onCancel,
}: RevokeGrantDialogProps) {
  return (
    <AlertDialog
      open={grant !== null}
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Revoke this standing grant?</AlertDialogTitle>
          <AlertDialogDescription>
            {grant
              ? `Future "${grant.action}" attempts on ${grant.domain} will trigger an approval card again. Existing runs are not affected.`
              : 'This grant goes back to requiring per-action approval.'}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={isRevoking}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            disabled={isRevoking || grant === null}
            onClick={(event) => {
              event.preventDefault()
              if (grant) onConfirm(grant)
            }}
            className={cn(buttonVariants({ variant: 'destructive' }))}
          >
            {isRevoking ? 'Revoking…' : 'Revoke grant'}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
