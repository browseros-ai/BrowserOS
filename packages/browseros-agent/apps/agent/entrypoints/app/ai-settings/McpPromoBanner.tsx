import { ArrowRight, Server, X } from 'lucide-react'
import { type FC, useState } from 'react'
import { useNavigate } from 'react-router'
import { Button } from '@/components/ui/button'
import { MCP_PROMO_BANNER_CLICKED_EVENT } from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'

export const McpPromoBanner: FC = () => {
  const [dismissed, setDismissed] = useState(false)
  const navigate = useNavigate()

  if (dismissed) return null

  const handleClick = () => {
    track(MCP_PROMO_BANNER_CLICKED_EVENT)
    navigate('/settings/mcp')
  }

  return (
    <div className="flex items-center gap-4 rounded-xl border border-border bg-card p-4 shadow-sm transition-all hover:shadow-md">
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-[var(--accent-orange)]/10">
        <Server className="h-5 w-5 text-[var(--accent-orange)]" />
      </div>
      <div className="min-w-0 flex-1">
        <p className="flex items-center gap-2 font-semibold text-sm">
          Use BrowserOS with Claude Code, Cursor & more
          <span className="text-[var(--accent-orange)] text-xs">
            (66+ tools)
          </span>
          <span className="inline-flex items-center gap-1 rounded-full bg-[var(--accent-orange)]/10 px-2.5 py-1 font-semibold text-[var(--accent-orange)] text-xs">
            <span className="h-1.5 w-1.5 rounded-full bg-[var(--accent-orange)]" />
            New
          </span>
        </p>
        <p className="text-muted-foreground text-xs">
          Connect your favorite coding tools to BrowserOS as an MCP server
        </p>
      </div>
      <Button
        variant="outline"
        size="sm"
        onClick={handleClick}
        className="shrink-0 border-[var(--accent-orange)] bg-[var(--accent-orange)]/10 text-[var(--accent-orange)] hover:bg-[var(--accent-orange)]/20 hover:text-[var(--accent-orange)]"
      >
        Set up
        <ArrowRight className="ml-1 h-3 w-3" />
      </Button>
      <button
        type="button"
        onClick={() => setDismissed(true)}
        className="shrink-0 rounded-sm p-1 text-muted-foreground opacity-50 transition-opacity hover:opacity-100"
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  )
}
