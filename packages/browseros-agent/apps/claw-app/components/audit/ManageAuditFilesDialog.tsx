/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * "Manage audit files" dialog: shows how much disk the audit content occupies
 * and lets the user choose an automatic retention window (7 / 30 days / custom /
 * never). Selecting an option persists it immediately; "Clean up now" applies
 * the current policy after a confirmation gate.
 */

import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
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
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import {
  useAuditStorage,
  useRunAuditCleanup,
  useSessions,
  useSetAuditRetention,
} from '@/modules/api/audit.hooks'
import {
  choiceFromRetention,
  cleanupConfirmText,
  formatBytes,
  type RetentionChoice,
  retentionRequest,
} from './manage-audit-files.helpers'

export function ManageAuditFilesDialog() {
  const storage = useAuditStorage()
  const queryClient = useQueryClient()
  const [open, setOpen] = useState(false)
  const invalidateStorage = () =>
    queryClient.invalidateQueries({ queryKey: useAuditStorage.getKey() })
  const setRetention = useSetAuditRetention({ onSuccess: invalidateStorage })
  const runCleanup = useRunAuditCleanup({
    onSuccess: () => {
      // Refresh both the usage numbers and the audit table the cleanup pruned.
      invalidateStorage()
      void queryClient.invalidateQueries({ queryKey: useSessions.getKey() })
    },
  })

  const server = choiceFromRetention(
    storage.data?.retention ?? { mode: 'keepForever' },
  )
  const [pendingChoice, setPendingChoice] = useState<RetentionChoice | null>(
    null,
  )
  const [customDays, setCustomDays] = useState(server.days)
  const choice = pendingChoice ?? server.choice
  const days = pendingChoice === null ? server.days : customDays

  const save = (next: RetentionChoice, nextDays: number) =>
    setRetention.mutate(retentionRequest(next, nextDays))

  const onSelect = (next: RetentionChoice) => {
    setPendingChoice(next)
    if (next !== 'custom') save(next, days)
  }

  // Close the dialog immediately and hand feedback to a toast — the sweep can
  // take a while, so we don't block the UI on it.
  const onCleanup = () => {
    setOpen(false)
    toast.promise(runCleanup.mutateAsync(), {
      loading: 'Cleaning up old audit files…',
      success: (result) =>
        `Removed ${result.sessionsDeleted} session${
          result.sessionsDeleted === 1 ? '' : 's'
        } and reclaimed ${formatBytes(result.bytesReclaimed)}.`,
      error: 'Could not clean up audit files.',
    })
  }

  const usage = storage.data?.usage
  const retention = storage.data?.retention ?? { mode: 'keepForever' as const }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger
        render={
          <Button variant="outline" size="sm">
            Manage audit files
          </Button>
        }
      />
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Manage audit files</DialogTitle>
          <DialogDescription>
            Storage used by sessions, screenshots, and replays.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-2 rounded-lg border border-border-2 p-4">
          {storage.isPending ? (
            <div className="h-16 animate-pulse rounded bg-muted" />
          ) : storage.isError ? (
            <p className="text-destructive text-sm">
              Could not load audit storage usage.
            </p>
          ) : (
            <>
              <UsageRow label="Recordings" bytes={usage?.recordingBytes ?? 0} />
              <UsageRow
                label="Screenshots"
                bytes={usage?.screenshotBytes ?? 0}
              />
              <div className="mt-1 flex items-center justify-between border-border-2 border-t pt-2 font-semibold">
                <span>Total</span>
                <span>{formatBytes(usage?.totalBytes ?? 0)}</span>
              </div>
            </>
          )}
        </div>

        <div className="flex flex-col gap-2">
          <label className="font-medium text-sm" htmlFor="audit-retention">
            Automatically delete audit log older than
          </label>
          <Select
            value={choice}
            onValueChange={(value) => {
              if (value !== null) onSelect(value)
            }}
          >
            <SelectTrigger id="audit-retention">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="7" label="7 days">
                7 days
              </SelectItem>
              <SelectItem value="30" label="30 days">
                30 days
              </SelectItem>
              <SelectItem value="custom" label="Custom">
                Custom
              </SelectItem>
              <SelectItem value="never" label="Never">
                Never
              </SelectItem>
            </SelectContent>
          </Select>
          {choice === 'custom' && (
            <input
              type="number"
              min={1}
              aria-label="Custom retention days"
              className="w-full rounded-md border border-border-2 bg-transparent px-3 py-2 text-sm"
              value={days}
              onChange={(event) =>
                setCustomDays(Math.max(1, Number(event.target.value) || 1))
              }
              onBlur={() => save('custom', customDays)}
            />
          )}
          {setRetention.isError && (
            <p className="text-destructive text-sm">
              Could not save the retention policy.
            </p>
          )}
        </div>

        <AlertDialog>
          <AlertDialogTrigger
            render={
              <Button
                variant="destructive"
                size="sm"
                disabled={runCleanup.isPending}
              >
                {runCleanup.isPending ? 'Cleaning up…' : 'Clean up now'}
              </Button>
            }
          />
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Clean up audit data now?</AlertDialogTitle>
              <AlertDialogDescription>
                {cleanupConfirmText(retention)}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancel</AlertDialogCancel>
              <AlertDialogAction onClick={onCleanup}>
                Delete and reclaim
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </DialogContent>
    </Dialog>
  )
}

function UsageRow({ label, bytes }: { label: string; bytes: number }) {
  return (
    <div className="flex items-center justify-between text-muted-foreground text-sm">
      <span>{label}</span>
      <span>{formatBytes(bytes)}</span>
    </div>
  )
}
