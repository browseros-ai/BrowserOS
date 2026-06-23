import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router'
import { Spinner } from '@/components/ui/spinner'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { type RunRow, useRuns } from '@/modules/api/runs.hooks'
import { AuditRow } from './AuditRow'
import {
  AUDIT_FILTERS,
  type AuditFilter,
  filterRuns,
} from './governance.helpers'

export function AuditTab() {
  const { data: runs, isLoading } = useRuns()
  const [filter, setFilter] = useState<AuditFilter>('all')
  const navigate = useNavigate()

  const visible = useMemo(() => filterRuns(runs ?? [], filter), [runs, filter])

  const onReplay = (run: RunRow) => {
    navigate(`/governance/audit/${run.id}/replay`)
  }

  return (
    <section className="flex flex-col gap-3">
      <header className="flex items-start gap-4">
        <p className="flex-1 text-ink-2 text-sm leading-snug">
          Every run an agent has driven, recorded and replayable. Closing the
          window loses nothing.
        </p>
        <ToggleGroup
          value={[filter]}
          onValueChange={(values) => {
            const next = values[0] as AuditFilter | undefined
            if (next) setFilter(next)
          }}
          spacing={0}
          variant="outline"
          className="bg-bg-sunken p-0.5"
        >
          {AUDIT_FILTERS.map(({ key, label }) => (
            <ToggleGroupItem
              key={key}
              value={key}
              className="h-7 rounded-md border-none bg-transparent px-3 font-semibold text-ink-3 text-xs shadow-none hover:bg-transparent hover:text-ink aria-pressed:bg-card aria-pressed:text-ink aria-pressed:shadow-sm"
            >
              {label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </header>

      <div className="flex items-center justify-between px-1 text-ink-3 text-xs">
        <span>Recent activity</span>
        <span>
          {visible.length} run{visible.length === 1 ? '' : 's'}
        </span>
      </div>

      {isLoading ? (
        <div className="flex justify-center py-12 text-ink-3">
          <Spinner />
        </div>
      ) : visible.length === 0 ? (
        <div className="rounded-xl border border-border border-dashed bg-card py-10 text-center text-ink-3 text-sm">
          No runs match this filter yet.
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          {visible.map((run) => (
            <AuditRow key={run.id} run={run} onReplay={onReplay} />
          ))}
        </div>
      )}
    </section>
  )
}
