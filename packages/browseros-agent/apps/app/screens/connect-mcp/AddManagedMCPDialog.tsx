import { KeyRound, Plus, Search } from 'lucide-react'
import { type FC, useState } from 'react'
import { McpServerIcon } from '@/components/mcp/McpServerIcon'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'

export interface UnauthenticatedServer {
  name: string
  description: string
}

export interface AddManagedMCPDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  serversList?: { name: string; description: string }[]
  unauthenticatedServers: UnauthenticatedServer[]
  onAddServer: (args: { name: string; description: string }) => void
  onAuthenticate: (serverName: string) => void
}

export const AddManagedMCPDialog: FC<AddManagedMCPDialogProps> = ({
  open,
  onOpenChange,
  serversList,
  unauthenticatedServers,
  onAddServer,
  onAuthenticate,
}) => {
  const [search, setSearch] = useState('')

  const handleAddServer = (args: { name: string; description: string }) => {
    onAddServer(args)
    onOpenChange(false)
  }

  const handleOpenChange = (isOpen: boolean) => {
    if (!isOpen) setSearch('')
    onOpenChange(isOpen)
  }

  const query = search.toLowerCase()
  const filteredUnauthenticated = unauthenticatedServers.filter(
    (s) =>
      s.name.toLowerCase().includes(query) ||
      s.description.toLowerCase().includes(query),
  )
  const filteredAvailable = serversList?.filter(
    (s) =>
      s.name.toLowerCase().includes(query) ||
      s.description.toLowerCase().includes(query),
  )

  const hasResults =
    filteredUnauthenticated.length > 0 || (filteredAvailable?.length ?? 0) > 0

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Add built-in app</DialogTitle>
          <DialogDescription>
            Select a built-in app to connect
          </DialogDescription>
        </DialogHeader>

        <div className="relative">
          <Search className="absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            placeholder="Search apps..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="pl-9"
          />
        </div>

        <div className="styled-scrollbar max-h-[400px] w-full space-y-4 overflow-y-auto">
          {filteredUnauthenticated.length > 0 && (
            <div className="space-y-2">
              <p className="px-1 font-medium text-muted-foreground text-xs uppercase tracking-wider">
                Needs authentication
              </p>
              {filteredUnauthenticated.map((server) => (
                <Button
                  key={server.name}
                  variant="outline"
                  onClick={() => onAuthenticate(server.name)}
                  className="group h-auto w-full items-center gap-3 border-amber-500/30 bg-amber-500/5 p-3 hover:border-amber-500 hover:bg-amber-500/10"
                >
                  <McpServerIcon
                    serverName={server.name}
                    size={20}
                    className="shrink-0 text-muted-foreground"
                  />
                  <div className="flex w-[calc(100%-64px)] flex-1 flex-col items-start gap-1">
                    <span className="font-medium">{server.name}</span>
                    {server.description && (
                      <p className="line-clamp-1 w-full text-ellipsis text-left text-muted-foreground text-xs">
                        {server.description}
                      </p>
                    )}
                  </div>
                  <KeyRound className="h-4 w-4 shrink-0 text-amber-500" />
                </Button>
              ))}
            </div>
          )}

          {(filteredAvailable?.length ?? 0) > 0 && (
            <div className="space-y-2">
              {filteredUnauthenticated.length > 0 && (
                <p className="px-1 font-medium text-muted-foreground text-xs uppercase tracking-wider">
                  Available
                </p>
              )}
              {filteredAvailable?.map((args) => {
                const { name: serverName, description } = args
                return (
                  <Button
                    key={serverName}
                    variant="outline"
                    onClick={() => handleAddServer(args)}
                    className="group h-auto w-full items-center gap-3 p-3 hover:border-[var(--accent-orange)] hover:bg-[var(--accent-orange)]/5"
                  >
                    <McpServerIcon
                      serverName={serverName}
                      size={20}
                      className="shrink-0 text-muted-foreground"
                    />
                    <div className="flex w-[calc(100%-64px)] flex-1 flex-col items-start gap-1">
                      <span className="font-medium">{serverName}</span>
                      {description && (
                        <p className="line-clamp-1 w-full text-ellipsis text-left text-muted-foreground text-xs">
                          {description}
                        </p>
                      )}
                    </div>
                    <Plus className="h-4 w-4 shrink-0 text-muted-foreground" />
                  </Button>
                )
              })}
            </div>
          )}

          {!hasResults && (
            <p className="py-6 text-center text-muted-foreground text-sm">
              No apps found
            </p>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
