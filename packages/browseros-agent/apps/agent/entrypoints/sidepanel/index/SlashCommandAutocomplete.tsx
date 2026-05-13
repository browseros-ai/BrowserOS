import { useCallback } from 'react'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import { Popover, PopoverAnchor, PopoverContent } from '@/components/ui/popover'
import type { SlashCommand } from '@/lib/slash-commands/types'

interface SlashCommandAutocompleteProps {
  /** Whether the autocomplete is open */
  isOpen: boolean
  /** Current filter text (everything after `/`) */
  filterText: string
  /** All available commands */
  commands: SlashCommand[]
  /** Called when a command is selected */
  onSelect: (command: SlashCommand) => void
  /** Called when the user dismisses the autocomplete */
  onClose: () => void
  /** Ref to the textarea/input element for anchoring */
  anchorRef: React.RefObject<HTMLElement | null>
}

export function SlashCommandAutocomplete({
  isOpen,
  filterText,
  commands,
  onSelect,
  onClose,
  anchorRef,
}: SlashCommandAutocompleteProps) {
  const filtered = commands.filter((cmd) => {
    if (!filterText) return true
    const q = filterText.toLowerCase()
    return (
      cmd.name.toLowerCase().includes(q) ||
      cmd.description.toLowerCase().includes(q)
    )
  })

  const handleSelect = useCallback(
    (cmd: SlashCommand) => {
      onSelect(cmd)
    },
    [onSelect],
  )

  // Keyboard navigation handled by parent — this component is purely visual
  // The parent (ChatInput) intercepts keys when isOpen is true

  if (!isOpen) return null

  return (
    <Popover open={isOpen}>
      {/* @ts-expect-error — Radix virtualRef not in the type stubs but works at runtime */}
      <PopoverAnchor virtualRef={anchorRef as React.RefObject<HTMLElement>} />
      <PopoverContent
        className="w-72 p-0"
        side="top"
        align="start"
        onInteractOutside={onClose}
        onEscapeKeyDown={onClose}
      >
        <Command className="border-0 shadow-none">
          <CommandList>
            {filtered.length === 0 && (
              <CommandEmpty>No matching commands</CommandEmpty>
            )}
            <CommandGroup heading="Commands">
              {filtered.map((cmd) => (
                <CommandItem
                  key={cmd.name}
                  value={cmd.name}
                  onSelect={() => handleSelect(cmd)}
                  className="flex flex-col items-start gap-0.5 px-3 py-2"
                >
                  <div className="flex items-center gap-2">
                    <span className="font-medium font-mono text-[var(--accent-orange)] text-sm">
                      /{cmd.name}
                    </span>
                    <span className="text-muted-foreground text-xs">
                      {cmd.type === 'action'
                        ? '⚡'
                        : cmd.type === 'template'
                          ? '📄'
                          : '💬'}
                    </span>
                  </div>
                  <span className="text-muted-foreground text-xs">
                    {cmd.description}
                  </span>
                  <span className="font-mono text-[10px] text-muted-foreground/60">
                    {cmd.usage}
                  </span>
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
