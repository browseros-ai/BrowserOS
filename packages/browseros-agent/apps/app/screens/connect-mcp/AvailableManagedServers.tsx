import { ChevronDown, Loader2, Plus } from 'lucide-react'
import { type FC, useState } from 'react'
import { McpServerIcon } from '@/components/mcp/McpServerIcon'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'

export interface AvailableManagedServersProps {
  availableServers?: { name: string; description: string }[]
  onAddServer: (args: { name: string; description: string }) => void
  isLoading?: boolean
}

export const AvailableManagedServers: FC<AvailableManagedServersProps> = ({
  availableServers,
  onAddServer,
  isLoading,
}) => {
  const [isOpen, setIsOpen] = useState(true)

  return (
    <Collapsible open={isOpen} onOpenChange={setIsOpen}>
      <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
        <CollapsibleTrigger className="flex w-full cursor-pointer items-center justify-between hover:opacity-50">
          <div className="flex flex-col items-start">
            <h3 className="font-semibold text-lg">Built-in Apps</h3>
            <p className="text-muted-foreground text-sm">
              {isLoading
                ? 'Loading...'
                : `${availableServers?.length} apps available`}
            </p>
          </div>
          <ChevronDown
            className={`h-5 w-5 text-muted-foreground transition-transform duration-200 ${
              isOpen ? 'rotate-180' : ''
            }`}
          />
        </CollapsibleTrigger>

        <CollapsibleContent className="pt-4">
          {isLoading ? (
            <div className="flex items-center justify-center py-8">
              <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
            </div>
          ) : (
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {availableServers?.map((args) => {
                const { name: serverName, description } = args
                return (
                  <Button
                    key={serverName}
                    variant="outline"
                    onClick={() => onAddServer(args)}
                    className="group h-auto flex-col items-start gap-2 p-4 hover:border-[var(--accent-orange)]"
                  >
                    <div className="flex w-full items-center justify-between">
                      <div className="flex items-center gap-3">
                        <McpServerIcon
                          serverName={serverName}
                          size={20}
                          className="text-muted-foreground transition-colors group-hover:text-[var(--accent-orange)]"
                        />
                        <span className="font-medium">{serverName}</span>
                      </div>
                      <Plus className="h-4 w-4 text-muted-foreground transition-colors group-hover:text-[var(--accent-orange)]" />
                    </div>
                    {description && (
                      <p className="line-clamp-1 max-w-48 text-ellipsis text-left text-muted-foreground text-xs">
                        {description}
                      </p>
                    )}
                  </Button>
                )
              }) ?? null}
            </div>
          )}
        </CollapsibleContent>
      </div>
    </Collapsible>
  )
}
