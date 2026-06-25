import { type FC, useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router'
import { RunResultDialog } from '@/components/ai-elements/run-result-dialog'
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import {
  NEW_SCHEDULED_TASK_CREATED_EVENT,
  SCHEDULED_TASK_CANCELLED_EVENT,
  SCHEDULED_TASK_DELETED_EVENT,
  SCHEDULED_TASK_EDITED_EVENT,
  SCHEDULED_TASK_RETRIED_EVENT,
  SCHEDULED_TASK_TESTED_EVENT,
  SCHEDULED_TASK_TOGGLED_EVENT,
  SCHEDULED_TASK_VIEW_RESULTS_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import {
  scheduledJobRunStorage,
  useScheduledJobRuns,
  useScheduledJobs,
} from '@/lib/schedules/scheduleStorage'
import type { ScheduledJobRun } from '@/lib/schedules/scheduleTypes'
import { NewScheduledTaskDialog } from './NewScheduledTaskDialog'
import { ScheduledTaskResults } from './ScheduledTaskResults'
import { ScheduledTasksHeader } from './ScheduledTasksHeader'
import { ScheduledTasksList } from './ScheduledTasksList'
import type { ScheduledJob } from './types'

/**
 * Main page for managing scheduled tasks
 * @public
 */
export const ScheduledTasksPage: FC = () => {
  const { jobs, addJob, editJob, toggleJob, removeJob, runJob } =
    useScheduledJobs()
  const { jobRuns, cancelJobRun } = useScheduledJobRuns()

  const [activeTab, setActiveTab] = useState<string | null>(null)
  const [isDialogOpen, setIsDialogOpen] = useState(false)
  const [editingJob, setEditingJob] = useState<ScheduledJob | null>(null)
  const [deleteJobId, setDeleteJobId] = useState<string | null>(null)
  const [viewingRunId, setViewingRunId] = useState<string | null>(null)
  const viewingRun = viewingRunId
    ? (jobRuns.find((r) => r.id === viewingRunId) ?? null)
    : null

  const [prefillValues, setPrefillValues] = useState<ScheduledJob | null>(null)
  const [searchParams, setSearchParams] = useSearchParams()
  const prefillHandled = useRef(false)

  useEffect(() => {
    if (prefillHandled.current) return
    if (searchParams.get('openDialog') !== 'true') return
    prefillHandled.current = true

    const prefill: ScheduledJob = {
      id: '',
      name: searchParams.get('name') ?? '',
      query: searchParams.get('query') ?? '',
      scheduleType:
        (searchParams.get('scheduleType') as ScheduledJob['scheduleType']) ??
        'daily',
      scheduleTime: searchParams.get('scheduleTime') ?? '09:00',
      scheduleInterval: 1,
      enabled: true,
      createdAt: '',
      updatedAt: '',
    }
    setPrefillValues(prefill)
    setEditingJob(null)
    setIsDialogOpen(true)
    setSearchParams({}, { replace: true })
  }, [searchParams, setSearchParams])

  const handleAdd = () => {
    setEditingJob(null)
    setIsDialogOpen(true)
  }

  const handleEdit = (job: ScheduledJob) => {
    setEditingJob(job)
    setIsDialogOpen(true)
  }

  const handleDelete = (jobId: string) => {
    setDeleteJobId(jobId)
  }

  const confirmDelete = async () => {
    if (deleteJobId) {
      await removeJob(deleteJobId)
      setDeleteJobId(null)
      track(SCHEDULED_TASK_DELETED_EVENT)
    }
  }

  const handleSave = async (
    data: Omit<ScheduledJob, 'id' | 'createdAt' | 'updatedAt'>,
  ) => {
    if (editingJob) {
      await editJob(editingJob.id, data)
      track(SCHEDULED_TASK_EDITED_EVENT, {
        scheduleType: data.scheduleType,
        interval: data.scheduleInterval,
        time: data.scheduleTime,
      })
    } else {
      await addJob(data)
      setActiveTab('tasks')
      track(NEW_SCHEDULED_TASK_CREATED_EVENT, {
        scheduleType: data.scheduleType,
        interval: data.scheduleInterval,
        time: data.scheduleTime,
      })
    }
  }

  const handleToggle = async (jobId: string, enabled: boolean) => {
    await toggleJob(jobId, enabled)
    track(SCHEDULED_TASK_TOGGLED_EVENT)
  }

  const handleRun = async (jobId: string) => {
    await runJob(jobId)
    track(SCHEDULED_TASK_TESTED_EVENT)
  }

  const handleCancelRun = async (runId: string) => {
    await cancelJobRun(runId)
    track(SCHEDULED_TASK_CANCELLED_EVENT)
  }

  const handleRetryRun = async (jobId: string) => {
    await runJob(jobId)
    track(SCHEDULED_TASK_RETRIED_EVENT)
  }

  const handleViewRun = (run: ScheduledJobRun) => {
    setViewingRunId(run.id)
    track(SCHEDULED_TASK_VIEW_RESULTS_EVENT)
  }

  useEffect(() => {
    scheduledJobRunStorage.getValue().then((runs) => {
      setActiveTab(runs && runs.length > 0 ? 'results' : 'tasks')
    })
  }, [])

  const jobToDelete = deleteJobId
    ? jobs.find((j) => j.id === deleteJobId)
    : null

  return (
    <div className="fade-in slide-in-from-bottom-5 animate-in space-y-6 duration-500">
      <ScheduledTasksHeader onAddClick={handleAdd} />

      {activeTab && (
        <Tabs value={activeTab} onValueChange={setActiveTab}>
          <TabsList>
            <TabsTrigger value="results">Results</TabsTrigger>
            <TabsTrigger value="tasks">Scheduled Tasks</TabsTrigger>
          </TabsList>

          <TabsContent value="results">
            <ScheduledTaskResults
              onViewRun={handleViewRun}
              onCancelRun={handleCancelRun}
              onRetryRun={handleRetryRun}
            />
          </TabsContent>

          <TabsContent value="tasks">
            <ScheduledTasksList
              jobs={jobs}
              onEdit={handleEdit}
              onDelete={handleDelete}
              onToggle={handleToggle}
              onRun={handleRun}
              onViewRun={handleViewRun}
              onCancelRun={handleCancelRun}
              onRetryRun={handleRetryRun}
            />
          </TabsContent>
        </Tabs>
      )}

      <NewScheduledTaskDialog
        open={isDialogOpen}
        onOpenChange={(open) => {
          setIsDialogOpen(open)
          if (!open) setPrefillValues(null)
        }}
        initialValues={editingJob ?? prefillValues}
        onSave={handleSave}
      />

      <RunResultDialog
        run={viewingRun}
        jobName={
          viewingRun
            ? jobs.find((j) => j.id === viewingRun.jobId)?.name
            : undefined
        }
        onOpenChange={(open) => !open && setViewingRunId(null)}
        onCancelRun={handleCancelRun}
        onRetryRun={(jobId) => {
          handleRetryRun(jobId)
          setViewingRunId(null)
        }}
      />

      <AlertDialog
        open={deleteJobId !== null}
        onOpenChange={(open) => !open && setDeleteJobId(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Scheduled Task</AlertDialogTitle>
            <AlertDialogDescription>
              Delete "{jobToDelete?.name}"? This will also remove all run
              history for this task.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={confirmDelete}>
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
