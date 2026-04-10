import { Shield } from 'lucide-react'
import type { FC } from 'react'
import { Badge } from '@/components/ui/badge'

interface AdminDashboardHeaderProps {
  pendingCount: number
  runningCount: number
}

export const AdminDashboardHeader: FC<AdminDashboardHeaderProps> = ({
  pendingCount,
  runningCount,
}) => {
  return (
    <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
      <div className="flex items-start gap-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
          <Shield className="h-6 w-6 text-[var(--accent-orange)]" />
        </div>
        <div className="flex-1">
          <div className="mb-1 flex flex-wrap items-center gap-2">
            <h2 className="font-semibold text-xl">Governance</h2>
            {pendingCount > 0 && (
              <Badge className="gap-1.5 rounded-full bg-yellow-500/10 text-yellow-600">
                {pendingCount} pending
              </Badge>
            )}
            {runningCount > 0 && (
              <Badge className="gap-1.5 rounded-full">
                {runningCount} live
              </Badge>
            )}
          </div>
          <p className="text-muted-foreground text-sm">
            Control agent permissions and audit every action.
          </p>
        </div>
      </div>
    </div>
  )
}
