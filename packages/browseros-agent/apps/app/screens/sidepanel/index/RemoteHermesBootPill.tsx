import { AlertCircle, Loader2 } from 'lucide-react'
import type { FC } from 'react'

export interface RemoteHermesVmStatus {
  status: 'booting' | 'error'
  progress?: string
}

export interface RemoteHermesBootPillProps {
  vm: RemoteHermesVmStatus
}

// Worker VmPublicView.progress values, mapped to user-friendly labels.
const PROGRESS_LABEL: Record<string, string> = {
  idle: 'starting',
  creating_volume: 'allocating storage',
  creating_machine: 'creating machine',
  pulling_image: 'pulling image',
  booting: 'booting',
  healthchecking: 'running health checks',
  stopping: 'stopping',
  destroying: 'destroying',
}

export const RemoteHermesBootPill: FC<RemoteHermesBootPillProps> = ({ vm }) => {
  if (vm.status === 'error') {
    return (
      <div className="mx-4 mb-2 flex items-center gap-2 rounded-md border border-red-200 bg-red-50 px-3 py-2 text-red-700 text-sm dark:border-red-800 dark:bg-red-950 dark:text-red-300">
        <AlertCircle className="h-4 w-4 shrink-0" />
        <span>Remote Hermes VM is unavailable. Try again in a moment.</span>
      </div>
    )
  }
  const stage = vm.progress ? PROGRESS_LABEL[vm.progress] : undefined
  return (
    <div className="mx-4 mb-2 flex items-center gap-2 rounded-md border border-blue-200 bg-blue-50 px-3 py-2 text-blue-700 text-sm dark:border-blue-800 dark:bg-blue-950 dark:text-blue-300">
      <Loader2 className="h-4 w-4 shrink-0 animate-spin" />
      <span>
        Booting Remote Hermes VM
        {stage
          ? ` — ${stage}…`
          : ' — this can take up to a minute on cold start.'}
      </span>
    </div>
  )
}
