import { Check, Folder, FolderOpen, Globe, X } from 'lucide-react'
import type { FC, PropsWithChildren } from 'react'
import { useState } from 'react'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { getBrowserOSAdapter } from '@/lib/browseros/adapter'
import { cn } from '@/lib/utils'
import type { WorkspaceFolder } from '@/lib/workspace/workspace-storage'
import { useWorkspace } from '@/modules/workspace/workspace.hooks'

export interface WorkspaceSelectorProps {
  side?: 'top' | 'bottom' | 'left' | 'right'
}

export const WorkspaceSelector: FC<
  PropsWithChildren<WorkspaceSelectorProps>
> = ({ children, side = 'bottom' }) => {
  const [open, setOpen] = useState(false)
  const [filterText, setFilterText] = useState('')
  const {
    recentFolders,
    selectedFolder,
    selectFolder,
    addFolder,
    removeFolder,
    clearSelection,
  } = useWorkspace()

  const query = filterText.toLowerCase()
  const filteredFolders = recentFolders.filter(
    (f) =>
      f.name.toLowerCase().includes(query) ||
      f.path.toLowerCase().includes(query),
  )

  const handleChooseFolder = async () => {
    try {
      const adapter = getBrowserOSAdapter()
      const result = await adapter.choosePath({ type: 'folder' })

      if (!result) {
        return
      }

      const folder: WorkspaceFolder = {
        id: crypto.randomUUID(),
        name: result.name,
        path: result.path,
        addedAt: Date.now(),
      }

      await addFolder(folder)
      setOpen(false)
    } catch {
      // User cancelled or API not available - silently ignore
    }
  }

  const handleSelectFolder = async (folder: WorkspaceFolder) => {
    if (selectedFolder?.id === folder.id) {
      await clearSelection()
    } else {
      await selectFolder(folder)
    }
    setOpen(false)
  }

  const handleRemoveFolder = async (e: React.MouseEvent, folderId: string) => {
    e.stopPropagation()
    await removeFolder(folderId)
  }

  const handleUseDefault = async () => {
    await clearSelection()
    setOpen(false)
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent
        side={side}
        align="start"
        className="w-72 p-0"
        role="dialog"
        aria-label="Select workspace folder"
      >
        <Command
          className="[&_svg:not([class*='text-'])]:text-muted-foreground"
          shouldFilter={false}
        >
          <CommandInput
            placeholder="Search folders..."
            className="h-9"
            value={filterText}
            onValueChange={setFilterText}
          />
          <CommandList className="max-h-64 overflow-auto">
            <CommandGroup>
              <CommandItem
                value="no-workspace"
                onSelect={handleUseDefault}
                className="flex items-center gap-3 px-3 py-2"
              >
                <Globe className="h-4 w-4 shrink-0 text-muted-foreground" />
                <div className="min-w-0 flex-1">
                  <span className="block text-sm">No workspace</span>
                  <span className="block text-muted-foreground text-xs">
                    AI works with tabs only
                  </span>
                </div>
                {!selectedFolder && (
                  <Check className="h-4 w-4 shrink-0 text-[var(--accent-orange)]" />
                )}
              </CommandItem>
            </CommandGroup>

            {filteredFolders.length > 0 && (
              <CommandGroup>
                <div className="my-2 px-2 font-semibold text-muted-foreground text-xs uppercase tracking-wide">
                  Recent
                </div>
                {filteredFolders.map((folder) => (
                  <CommandItem
                    key={folder.id}
                    value={`${folder.id} ${folder.name} ${folder.path}`}
                    onSelect={() => handleSelectFolder(folder)}
                    className="group flex items-center gap-3 px-3 py-2"
                  >
                    <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
                    <div className="min-w-0 flex-1">
                      <span className="block truncate font-medium text-sm">
                        {folder.name}
                      </span>
                      <span className="block truncate text-muted-foreground text-xs">
                        {folder.path}
                      </span>
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      {selectedFolder?.id === folder.id && (
                        <Check className="h-4 w-4 text-[var(--accent-orange)]" />
                      )}
                      <button
                        type="button"
                        onClick={(e) => handleRemoveFolder(e, folder.id)}
                        className={cn(
                          'rounded p-0.5 transition-opacity hover:bg-muted-foreground/20',
                          'opacity-0 group-hover:opacity-100',
                        )}
                        aria-label={`Remove ${folder.name} from recents`}
                      >
                        <X className="h-3 w-3" />
                      </button>
                    </div>
                  </CommandItem>
                ))}
              </CommandGroup>
            )}

            <CommandEmpty>No folders found</CommandEmpty>
          </CommandList>

          <div className="border-border/50 border-t">
            <button
              type="button"
              onClick={handleChooseFolder}
              className="flex w-full items-center gap-3 px-3 py-2.5 text-left text-sm transition-colors hover:bg-muted"
            >
              <FolderOpen className="h-4 w-4 text-muted-foreground" />
              <span>Choose a different folder</span>
            </button>
          </div>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
