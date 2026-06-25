import { defineExtensionMessaging } from '@webext-core/messaging'

interface RunScheduledJobData {
  jobId: string
}

interface CancelScheduledJobRunData {
  runId: string
}

interface RunScheduledJobResponse {
  success: boolean
  error?: string
}

type ScheduleMessagesProtocol = {
  runScheduledJob(data: RunScheduledJobData): RunScheduledJobResponse
  cancelScheduledJobRun(
    data: CancelScheduledJobRunData,
  ): RunScheduledJobResponse
}

const { sendMessage, onMessage } =
  defineExtensionMessaging<ScheduleMessagesProtocol>()

export { onMessage as onScheduleMessage, sendMessage as sendScheduleMessage }
