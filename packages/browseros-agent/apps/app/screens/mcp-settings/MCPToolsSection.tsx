import { ChevronDown, Loader2, RefreshCw, Wrench } from 'lucide-react'
import { type FC, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import type { McpTool } from '@/lib/mcp/client'

export interface MCPToolsSectionProps {
  tools: McpTool[]
  isLoading: boolean
  error: string | null
  onRefresh: () => void
}

export const MCPToolsSection: FC<MCPToolsSectionProps> = ({
  tools,
  isLoading,
  error,
  onRefresh,
}) => {
  const [isOpen, setIsOpen] = useState(false)

  return (
    <Collapsible
      open={isOpen}
      onOpenChange={setIsOpen}
      className="group/collapsible"
    >
      <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
        <div className="flex items-start justify-between gap-3">
          <CollapsibleTrigger className="flex flex-1 items-center justify-between text-left">
            <div>
              <h3 className="font-semibold text-lg">Available Tools</h3>
              {tools.length > 0 && (
                <p className="text-muted-foreground text-sm">
                  {tools.length} tools available
                </p>
              )}
            </div>
            <ChevronDown
              className={`h-5 w-5 text-muted-foreground transition-transform duration-200 ${
                isOpen ? 'rotate-180' : ''
              }`}
            />
          </CollapsibleTrigger>
          <Button
            variant="outline"
            size="icon"
            onClick={onRefresh}
            disabled={isLoading}
            className="border-[var(--accent-orange)] bg-[var(--accent-orange)]/10 text-[var(--accent-orange)] hover:bg-[var(--accent-orange)]/20 hover:text-[var(--accent-orange)]"
            title="Refresh tools"
          >
            {isLoading ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <RefreshCw className="h-4 w-4" />
            )}
          </Button>
        </div>

        {error && (
          <div className="mt-4 rounded-lg border border-destructive/20 bg-destructive/10 px-4 py-3 text-destructive text-sm">
            {error}
          </div>
        )}

        <CollapsibleContent className="pt-4">
          {tools.length > 0 && (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {tools.map((tool) => (
                <div
                  key={tool.name}
                  className="rounded-lg border border-border bg-background p-4 transition-all hover:border-[var(--accent-orange)]/50 hover:shadow-sm"
                >
                  <div className="mb-2 flex items-start gap-2">
                    <Wrench className="mt-0.5 h-4 w-4 shrink-0 text-[var(--accent-orange)]" />
                    <span className="min-w-0 break-words font-medium text-sm">
                      {tool.name}
                    </span>
                  </div>
                  {tool.description && (
                    <p className="line-clamp-2 break-words text-muted-foreground text-xs">
                      {tool.description}
                    </p>
                  )}
                </div>
              ))}
            </div>
          )}
        </CollapsibleContent>
      </div>
    </Collapsible>
  )
}
