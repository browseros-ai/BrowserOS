import { AlertTriangle } from 'lucide-react'
import type { FC } from 'react'
import { ConfigGroup } from './ConfigGroup'
import type { ConfigKeySchema } from './config-queries'

interface DangerousSectionProps {
  open: boolean
  onToggle: () => void
  fields: ConfigKeySchema[]
  values: Record<string, string>
  currentValues: Record<string, number>
  defaults: Record<string, number>
  errors: Record<string, string>
  onValueChange: (key: string, value: string) => void
  onReset: (key: string) => void
}

export const DangerousSection: FC<DangerousSectionProps> = (props) => {
  return (
    <div className="rounded-xl border border-amber-500/30 bg-amber-500/5">
      <div className="px-4 pt-4 text-amber-800 dark:text-amber-200">
        <div className="mb-1 flex items-center gap-2 font-semibold text-sm">
          <AlertTriangle className="size-4" />
          Dangerous settings
        </div>
        <p className="text-xs opacity-80">
          These values can degrade stability, performance, or auth flows. Edit
          only if you understand the risk notes.
        </p>
      </div>
      <div className="p-2 pt-3">
        <ConfigGroup {...props} title="Dangerous" />
      </div>
    </div>
  )
}
