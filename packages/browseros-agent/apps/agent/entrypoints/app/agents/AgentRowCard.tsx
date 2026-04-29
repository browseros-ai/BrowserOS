import {
  Copy,
  Loader2,
  MessageSquare,
  MoreHorizontal,
  Pencil,
  RotateCcw,
  Trash2,
} from 'lucide-react'
import type { FC } from 'react'
import { useNavigate } from 'react-router'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { AdapterIcon, adapterLabel } from './AdapterIcon'
import {
  canDelete as canDeleteAgent,
  canRename as canRenameAgent,
  displayName,
  formatRelativeTime,
  workspaceLabel,
} from './agent-display.helpers'
import type { HarnessAgentAdapter } from './agent-harness-types'
import type { AgentListItem } from './agents-page-types'
import { type AgentLiveness, LivenessDot } from './LivenessDot'

interface AgentRowCardProps {
  agent: AgentListItem
  /**
   * Per-agent extras the listing surface provides on top of the
   * minimal `AgentListItem` shape. `lastUsedAt` survives server
   * restart (sourced from acpx session record); `status` is in-memory
   * server-side.
   */
  status?: AgentLiveness
  lastUsedAt?: number | null
  /** Adapter the agent belongs to. Drives icon + label. */
  adapter?: HarnessAgentAdapter
  /** Reasoning effort chip (claude/codex/openclaw catalog). */
  reasoningEffort?: string | null
  /** Modeled directly off the inbound delete handler so the parent owns the dialog. */
  onDelete: (agent: AgentListItem) => void
  /** Whether THIS agent is mid-delete; renders a spinner in place of the trash icon. */
  deleting?: boolean
}

export const AgentRowCard: FC<AgentRowCardProps> = ({
  agent,
  status = 'unknown',
  lastUsedAt,
  adapter,
  reasoningEffort,
  onDelete,
  deleting,
}) => {
  const navigate = useNavigate()
  const adapterId = adapter ?? inferAdapterFromListItem(agent)
  const workspace = workspaceLabel(agent)
  const lastUsedLabel = formatRelativeTime(lastUsedAt ?? null)
  const allowDelete = canDeleteAgent(agent)
  const allowRename = canRenameAgent(agent)

  const handleChat = () => navigate(`/agents/${agent.agentId}`)
  const handleCopyId = async () => {
    try {
      await navigator.clipboard.writeText(agent.agentId)
      toast.success('Agent id copied')
    } catch {
      toast.error('Could not copy agent id')
    }
  }

  return (
    <div
      className={cn(
        'group rounded-xl border border-border bg-card p-4 shadow-sm transition-all',
        'hover:border-[var(--accent-orange)]/50 hover:shadow-sm',
      )}
    >
      <div className="flex items-start gap-4">
        {/* Adapter tile + liveness dot in the corner. */}
        <div className="relative shrink-0">
          <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-muted text-muted-foreground">
            <AdapterIcon adapter={adapterId} className="h-6 w-6" />
          </div>
          <LivenessDot
            status={status}
            detail={livenessDetail(status, lastUsedAt)}
            className="absolute -right-0.5 -bottom-0.5"
          />
        </div>

        <div className="min-w-0 flex-1">
          <div className="mb-1 flex items-center gap-2">
            <span className="truncate font-semibold">{displayName(agent)}</span>
            {status === 'working' && (
              <Badge
                variant="secondary"
                className="bg-amber-50 text-amber-900 hover:bg-amber-50"
              >
                Working
              </Badge>
            )}
            {status === 'asleep' && (
              <Badge variant="outline" className="text-muted-foreground">
                Asleep
              </Badge>
            )}
            {status === 'error' && (
              <Badge variant="destructive">Attention</Badge>
            )}
          </div>

          <div className="mb-2 flex flex-wrap items-center gap-1.5 text-xs">
            <Badge variant="secondary" className="font-normal">
              {adapterLabel(adapterId)}
            </Badge>
            {agent.modelLabel && agent.modelLabel !== 'default' && (
              <Badge variant="outline" className="font-normal">
                {agent.modelLabel}
              </Badge>
            )}
            {reasoningEffort && reasoningEffort !== 'medium' && (
              <Badge variant="outline" className="font-normal">
                {reasoningEffort}
              </Badge>
            )}
          </div>

          <div className="flex flex-wrap items-center gap-2 text-muted-foreground text-xs">
            <span>Last used {lastUsedLabel}</span>
            {workspace && (
              <>
                <span aria-hidden>•</span>
                <span className="truncate font-mono" title={workspace}>
                  {workspace}
                </span>
              </>
            )}
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-2">
          <Button variant="outline" size="sm" onClick={handleChat}>
            <MessageSquare className="mr-1.5 h-3 w-3" />
            Chat
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                aria-label={`More actions for ${displayName(agent)}`}
                className="h-8 w-8"
              >
                <MoreHorizontal className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-44">
              <DropdownMenuItem onSelect={() => void handleCopyId()}>
                <Copy className="mr-2 h-3.5 w-3.5" />
                Copy id
              </DropdownMenuItem>
              <RenameMenuItem disabled={!allowRename} />
              <ResetHistoryMenuItem />
              <DropdownMenuSeparator />
              <DropdownMenuItem
                onSelect={() => onDelete(agent)}
                disabled={!allowDelete || deleting}
                className="text-destructive focus:text-destructive"
              >
                {deleting ? (
                  <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Trash2 className="mr-2 h-3.5 w-3.5" />
                )}
                Delete
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
    </div>
  )
}

const RenameMenuItem: FC<{ disabled: boolean }> = ({ disabled }) => {
  const item = (
    <DropdownMenuItem disabled className="text-muted-foreground">
      <Pencil className="mr-2 h-3.5 w-3.5" />
      Rename
    </DropdownMenuItem>
  )
  if (!disabled) return item
  // Disabled but with a hint so users know it's coming, not broken.
  return (
    <TooltipProvider delayDuration={300}>
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="block w-full">{item}</span>
        </TooltipTrigger>
        <TooltipContent side="left" className="text-xs">
          Rename coming soon
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}

const ResetHistoryMenuItem: FC = () => {
  const item = (
    <DropdownMenuItem disabled className="text-muted-foreground">
      <RotateCcw className="mr-2 h-3.5 w-3.5" />
      Reset history
    </DropdownMenuItem>
  )
  return (
    <TooltipProvider delayDuration={300}>
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="block w-full">{item}</span>
        </TooltipTrigger>
        <TooltipContent side="left" className="text-xs">
          Reset history coming soon
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}

function inferAdapterFromListItem(
  agent: AgentListItem,
): HarnessAgentAdapter | 'unknown' {
  const label = agent.runtimeLabel?.toLowerCase()
  if (label?.includes('claude')) return 'claude'
  if (label?.includes('codex')) return 'codex'
  if (label?.includes('openclaw')) return 'openclaw'
  return 'unknown'
}

function livenessDetail(
  status: AgentLiveness,
  lastUsedAt: number | null | undefined,
): string | undefined {
  if (lastUsedAt == null) return undefined
  const diffMin = Math.floor((Date.now() - lastUsedAt) / 60_000)
  if (status === 'idle') return `Idle for ${Math.max(0, diffMin)} min`
  if (status === 'asleep') {
    if (diffMin < 60) return `Asleep — quiet for ${diffMin} min`
    const hr = Math.floor(diffMin / 60)
    return `Asleep — quiet for ${hr} hr`
  }
  if (status === 'working') return 'Working on a turn'
  if (status === 'error') return 'Attention — last turn failed'
  return undefined
}
