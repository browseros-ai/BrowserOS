import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
} from '@/components/motion/center-morph-modal'
import { Input } from '@/components/motion/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/motion/select'
import { Skeleton } from '@/components/ui/skeleton'
import {
  useAuditStorage,
  useRunAuditCleanup,
  useSessions,
  useSetAuditRetention,
} from '@/modules/api/audit.hooks'
import { toastPromise } from '@/modules/toast/toast'
import { SettingRow, SettingsSection } from './settings.components'
import {
  choiceFromRetention,
  cleanupConfirmText,
  formatBytes,
  type RetentionChoice,
  retentionRequest,
} from './storage.helpers'

const RETENTION_ID = 'retention'
const SPACE_ID = 'space-used'

const RETENTION_LABELS: Record<RetentionChoice, string> = {
  '7': '7 days',
  '30': '30 days',
  custom: 'Pick a number of days',
  never: 'Forever',
}

/** How long task history stays, how much room it takes, and clearing it out. */
export function StorageSettings() {
  const storage = useAuditStorage({ refetchInterval: 30000 })
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  const invalidateStorage = () =>
    queryClient.invalidateQueries({ queryKey: useAuditStorage.getKey() })
  const setRetention = useSetAuditRetention({ onSuccess: invalidateStorage })
  const runCleanup = useRunAuditCleanup({
    onSuccess: () => {
      // The cleanup pruned History too, so refresh both.
      invalidateStorage()
      void queryClient.invalidateQueries({ queryKey: useSessions.getKey() })
    },
  })

  const retention = storage.data?.retention ?? { mode: 'keepForever' as const }
  const server = choiceFromRetention(retention)
  const [pendingChoice, setPendingChoice] = useState<RetentionChoice | null>(
    null,
  )
  // Null until the person edits the field, so it shows the saved days even
  // when they load after the first render.
  const [editedDays, setEditedDays] = useState<string | null>(null)
  const customDays = editedDays ?? String(server.days)
  const choice = pendingChoice ?? server.choice

  const save = (next: RetentionChoice, days: number) =>
    setRetention.mutate(retentionRequest(next, days))

  const choose = (next: RetentionChoice) => {
    setPendingChoice(next)
    if (next === 'custom') setEditedDays(null)
    else save(next, server.days)
  }

  const saveCustom = () => {
    const days = Math.max(1, Math.floor(Number(customDays) || 1))
    setEditedDays(String(days))
    save('custom', days)
  }

  // The sweep can take a while, so the dialog closes and a toast follows it.
  const cleanUp = () => {
    setConfirming(false)
    toastPromise(runCleanup.mutateAsync(), {
      loading: 'Cleaning up old tasks…',
      success: (result) =>
        `Cleaned up ${result.sessionsDeleted} ${
          result.sessionsDeleted === 1 ? 'task' : 'tasks'
        } and freed ${formatBytes(result.bytesReclaimed)}.`,
      error: 'Couldn’t clean up. Try again.',
    })
  }

  const usage = storage.data?.usage

  return (
    <SettingsSection title="History and storage">
      <SettingRow
        id={RETENTION_ID}
        title="Keep task history for"
        description="Older tasks, pictures and recordings are deleted automatically."
        control={
          <Select
            value={choice}
            onValueChange={(value) => choose(value as RetentionChoice)}
            disabled={!storage.data || setRetention.isPending}
            className="w-56"
          >
            <SelectTrigger className="h-10 font-semibold text-[14px] text-ink-2">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {(Object.keys(RETENTION_LABELS) as RetentionChoice[]).map(
                (value) => (
                  <SelectItem key={value} value={value}>
                    {RETENTION_LABELS[value]}
                  </SelectItem>
                ),
              )}
            </SelectContent>
          </Select>
        }
      >
        {choice === 'custom' ? (
          <form
            className="flex flex-wrap items-center gap-2"
            onSubmit={(event) => {
              event.preventDefault()
              saveCustom()
            }}
          >
            <Input
              type="number"
              min={1}
              inputMode="numeric"
              aria-label="Days to keep task history"
              value={customDays}
              onChange={setEditedDays}
              className="w-28"
            />
            <span className="text-[14px] text-ink-2">days</span>
            <Button
              type="submit"
              variant="secondary"
              size="sm"
              disabled={setRetention.isPending}
              className="font-semibold"
            >
              Save
            </Button>
          </form>
        ) : null}
        {setRetention.isError ? (
          <p role="alert" className="text-[13.5px] text-red-ink">
            Couldn’t save how long to keep history. Try again.
          </p>
        ) : null}
      </SettingRow>

      <SettingRow
        id={SPACE_ID}
        title="Space used"
        description={
          storage.isPending ? (
            <Skeleton className="mt-1 h-4 w-64" />
          ) : storage.isError ? (
            'Couldn’t check how much space your history uses.'
          ) : (
            `${formatBytes(usage?.totalBytes ?? 0)} of pictures and recordings on this computer (${formatBytes(
              usage?.screenshotBytes ?? 0,
            )} of pictures, ${formatBytes(usage?.recordingBytes ?? 0)} of recordings).`
          )
        }
        control={
          <StatefulButton
            variant="secondary"
            size="sm"
            state={runCleanup.isPending ? 'loading' : 'idle'}
            loadingText="Cleaning up"
            disabled={runCleanup.isPending}
            onClick={() => setConfirming(true)}
            className="font-semibold"
          >
            Clean up now
          </StatefulButton>
        }
      />

      <CenterMorphModal open={confirming} onOpenChange={setConfirming}>
        <CenterMorphModalContent
          ariaLabel="Clean up task history now?"
          showCloseButton={false}
          className="ph-no-capture max-w-[440px] border-border-2 bg-card"
        >
          <div className="flex flex-col gap-5 p-6">
            <div className="flex flex-col gap-2">
              <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
                Clean up task history now?
              </h2>
              <p className="text-[14.5px] text-ink-2">
                {cleanupConfirmText(retention)}
              </p>
            </div>
            <div className="flex justify-end gap-2">
              <CenterMorphModalClose>
                <Button variant="ghost">Keep it</Button>
              </CenterMorphModalClose>
              <Button
                onClick={cleanUp}
                className="bg-red font-semibold text-white hover:bg-red/90"
              >
                Clean up
              </Button>
            </div>
          </div>
        </CenterMorphModalContent>
      </CenterMorphModal>
    </SettingsSection>
  )
}
