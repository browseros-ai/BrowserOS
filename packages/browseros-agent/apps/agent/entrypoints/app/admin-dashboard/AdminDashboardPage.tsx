import dayjs from 'dayjs'
import { Shield } from 'lucide-react'
import { type FC, useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { ExecutionTaskCard } from '@/components/execution-history/ExecutionTaskCard'
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
import {
  removeConversationExecutionTask,
  useExecutionHistoryByConversation,
} from '@/lib/execution-history/storage'
import type { ExecutionTaskRecord } from '@/lib/execution-history/types'
import { pendingToolApprovalsStorage } from '@/lib/tool-approvals/approval-sync-storage'
import { AdminDashboardHeader } from './AdminDashboardHeader'
import { PendingApprovals } from './PendingApprovals'

type TaskGroup = {
  label: string
  tasks: ExecutionTaskRecord[]
}

function getGroupLabel(date: string) {
  const startedAt = dayjs(date)
  if (startedAt.isSame(dayjs(), 'day')) return 'Today'
  if (startedAt.isSame(dayjs().subtract(1, 'day'), 'day')) return 'Yesterday'
  return startedAt.format('MMMM D, YYYY')
}

function groupTasks(tasks: ExecutionTaskRecord[]): TaskGroup[] {
  const grouped = new Map<string, ExecutionTaskRecord[]>()

  for (const task of tasks) {
    const label = getGroupLabel(task.startedAt)
    const existing = grouped.get(label) ?? []
    grouped.set(label, [...existing, task])
  }

  return Array.from(grouped.entries()).map(([label, groupItems]) => ({
    label,
    tasks: groupItems,
  }))
}

export const AdminDashboardPage: FC = () => {
  const [pendingCount, setPendingCount] = useState(0)
  const historyByConversation = useExecutionHistoryByConversation()
  const [taskToDelete, setTaskToDelete] = useState<ExecutionTaskRecord | null>(
    null,
  )

  useEffect(() => {
    pendingToolApprovalsStorage
      .getValue()
      .then((v) => setPendingCount(v.length))
    const unwatch = pendingToolApprovalsStorage.watch((v) =>
      setPendingCount(v.length),
    )
    return () => unwatch()
  }, [])

  const historyList = useMemo(
    () => Object.values(historyByConversation),
    [historyByConversation],
  )

  const tasks = useMemo(() => {
    return historyList
      .flatMap((history) => history.tasks)
      .sort(
        (left, right) =>
          new Date(right.startedAt).getTime() -
          new Date(left.startedAt).getTime(),
      )
  }, [historyList])

  const groupedTasks = useMemo(() => groupTasks(tasks), [tasks])
  const runningCount = useMemo(
    () => tasks.filter((task) => task.status === 'running').length,
    [tasks],
  )
  const conversationCount = historyList.length

  const handleDeleteTask = async () => {
    if (!taskToDelete) return

    try {
      await removeConversationExecutionTask({
        conversationId: taskToDelete.conversationId,
        taskId: taskToDelete.id,
      })
      toast.success('Run removed')
    } catch {
      toast.error('Failed to remove run')
    } finally {
      setTaskToDelete(null)
    }
  }

  return (
    <div className="fade-in slide-in-from-bottom-5 animate-in space-y-6 duration-500">
      <AdminDashboardHeader
        pendingCount={pendingCount}
        runningCount={runningCount}
      />

      <section className="space-y-3">
        <h3 className="font-semibold text-sm">Approvals</h3>
        <PendingApprovals />
      </section>

      <section className="space-y-4">
        <div>
          <h3 className="font-semibold text-sm">Audit Trail</h3>
          {tasks.length > 0 && (
            <p className="mt-1 text-muted-foreground text-sm">
              {tasks.length} recorded run{tasks.length === 1 ? '' : 's'}
              {conversationCount > 1
                ? ` across ${conversationCount} chats`
                : ''}
              . Newest first.
            </p>
          )}
        </div>

        {tasks.length === 0 ? (
          <div className="rounded-xl border border-dashed px-6 py-14 text-center">
            <div className="mx-auto mb-4 flex size-12 items-center justify-center rounded-2xl bg-[var(--accent-orange)]/10">
              <Shield className="size-5 text-[var(--accent-orange)]" />
            </div>
            <h3 className="mb-1 font-medium text-lg">No agent runs yet</h3>
            <p className="mx-auto max-w-sm text-muted-foreground text-sm">
              Run a task in BrowserOS and the execution history will appear
              here.
            </p>
          </div>
        ) : (
          <div className="space-y-6">
            {groupedTasks.map((group, groupIndex) => (
              <section key={group.label} className="space-y-3">
                <div className="flex items-center gap-3">
                  <h4 className="font-medium text-muted-foreground text-xs">
                    {group.label}
                  </h4>
                  <div className="h-px flex-1 bg-border/60" />
                  <span className="text-muted-foreground text-xs">
                    {group.tasks.length} run
                    {group.tasks.length === 1 ? '' : 's'}
                  </span>
                </div>
                <div className="space-y-3">
                  {group.tasks.map((task, index) => (
                    <ExecutionTaskCard
                      key={task.id}
                      task={task}
                      defaultOpen={
                        task.status === 'running' ||
                        (groupIndex === 0 && index === 0)
                      }
                      onDelete={setTaskToDelete}
                    />
                  ))}
                </div>
              </section>
            ))}
          </div>
        )}
      </section>

      <AlertDialog
        open={taskToDelete !== null}
        onOpenChange={(open) => !open && setTaskToDelete(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Run</AlertDialogTitle>
            <AlertDialogDescription>
              Remove "{taskToDelete?.promptText}" from local history? This only
              clears the recorded run on this device.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={handleDeleteTask}>
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
