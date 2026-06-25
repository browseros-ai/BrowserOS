import { Clock, Pencil, Trash2 } from 'lucide-react'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { AgentProfile } from '@/modules/api/agents.hooks'
import { scopeSummaryFor, statusMetaFor } from './agents.helpers'

interface AgentDirectoryRowProps {
  profile: AgentProfile
  onEdit: (profile: AgentProfile) => void
  onRevoke: (profile: AgentProfile) => void
}

/**
 * One row per configured agent profile. Harness icon chip on the
 * left, profile name + scope summary + last-run timestamp in the
 * middle, status pill + edit/revoke buttons on the right. The whole
 * row is non-clickable; explicit Edit / Revoke buttons stay focusable
 * so screen-reader users hit them in tab order.
 */
export function AgentDirectoryRow({
  profile,
  onEdit,
  onRevoke,
}: AgentDirectoryRowProps) {
  const statusMeta = statusMetaFor(profile.status)
  return (
    <div className="flex items-center gap-3.5 rounded-xl border border-border bg-card px-4 py-3.5">
      <span
        className={cn(
          'flex size-9 shrink-0 items-center justify-center rounded-lg',
          profile.harness === 'Codex'
            ? 'bg-bg-sunken text-ink-2'
            : 'bg-accent-tint text-accent',
        )}
      >
        <HarnessIcon harness={profile.harness} className="size-4" />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <h3 className="truncate font-bold text-ink text-sm">
            {profile.name}
          </h3>
          <span className="text-ink-4 text-xs">·</span>
          <span className="text-ink-3 text-xs">{profile.harness}</span>
        </div>
        <p className="mt-0.5 truncate text-ink-3 text-xs">
          {scopeSummaryFor(profile)}
        </p>
        <p className="mt-1 flex items-center gap-1 text-[11px] text-ink-4">
          <Clock className="size-3" />
          Last run {profile.lastRunAt}
        </p>
      </div>
      <Badge
        variant="outline"
        className={cn('font-bold text-[11px]', statusMeta.className)}
      >
        {statusMeta.label}
      </Badge>
      <div className="flex gap-1.5">
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => onEdit(profile)}
          className="h-8 gap-1.5 px-2.5 text-xs"
        >
          <Pencil className="size-3" />
          Edit
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => onRevoke(profile)}
          aria-label={`Revoke ${profile.name}`}
          className="h-8 px-2.5 text-ink-3 hover:text-red"
        >
          <Trash2 className="size-3.5" />
        </Button>
      </div>
    </div>
  )
}
