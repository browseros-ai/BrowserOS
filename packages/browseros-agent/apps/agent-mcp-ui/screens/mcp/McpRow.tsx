import { Check, Copy, RotateCcw } from 'lucide-react'
import { useState } from 'react'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { AgentProfile } from '@/modules/api/agents.hooks'
import { statusMetaFor } from '@/screens/agents/agents.helpers'
import { cliCommandFor, slugFromMcpUrl } from './mcp.helpers'

interface McpRowProps {
  profile: AgentProfile
  isRegenerating: boolean
  onRegenerate: (profile: AgentProfile) => void
}

export function McpRow({ profile, isRegenerating, onRegenerate }: McpRowProps) {
  const statusMeta = statusMetaFor(profile.status)
  const slug = slugFromMcpUrl(profile.mcpUrl)
  const cliCommand = cliCommandFor(profile)
  const [copied, setCopied] = useState(false)

  const copyMcpUrl = async () => {
    try {
      await navigator.clipboard.writeText(profile.mcpUrl)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      setCopied(false)
    }
  }

  return (
    <div className="flex flex-col gap-3 rounded-2xl border border-border bg-card p-4">
      <div className="flex items-center gap-3.5">
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
            slug <span className="font-mono text-ink-2">{slug}</span> · CLI{' '}
            <span className="font-mono text-ink-2">{cliCommand}</span>
          </p>
        </div>
        <Badge
          variant="outline"
          className={cn('font-bold text-[11px]', statusMeta.className)}
        >
          {statusMeta.label}
        </Badge>
      </div>

      <div className="flex items-center gap-2 rounded-lg bg-[#15140F] px-3 py-2">
        <code className="min-w-0 flex-1 truncate font-mono text-[#EDEAE2] text-[11.5px]">
          {profile.mcpUrl}
        </code>
        <button
          type="button"
          onClick={copyMcpUrl}
          aria-label="Copy MCP URL"
          className="flex size-6 items-center justify-center rounded-md bg-white/10 text-white hover:bg-white/20"
        >
          {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
        </button>
      </div>

      <div className="flex gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => onRegenerate(profile)}
          disabled={isRegenerating}
          className="gap-1.5"
        >
          <RotateCcw
            className={cn('size-3.5', isRegenerating && 'animate-spin')}
          />
          {isRegenerating ? 'Rotating…' : 'Regenerate URL'}
        </Button>
      </div>
    </div>
  )
}
