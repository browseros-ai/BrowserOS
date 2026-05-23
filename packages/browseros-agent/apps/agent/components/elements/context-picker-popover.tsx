import type * as React from 'react'
import type { FC, PropsWithChildren } from 'react'
import { useEffect, useMemo, useRef, useState } from 'react'
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
  PopoverAnchor,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { ContextListItem } from './context-list-item'
import { useContextSources, type ContextItem } from './use-context-sources'

type PopoverSide = 'top' | 'bottom' | 'left' | 'right'

interface ContextPickerCommonProps {
  selectedItems: ContextItem[]
  onToggleItem: (item: ContextItem) => void
}

interface ContextPickerMentionPopoverProps extends ContextPickerCommonProps {
  variant: 'mention'
  isOpen: boolean
  filterText: string
  onClose: () => void
  anchorRef: React.RefObject<HTMLElement | null>
  side?: PopoverSide
}

interface ContextPickerSelectorPopoverProps
  extends PropsWithChildren<ContextPickerCommonProps> {
  variant: 'selector'
  side?: PopoverSide
}

export type ContextPickerPopoverProps =
  | ContextPickerMentionPopoverProps
  | ContextPickerSelectorPopoverProps

/**
 * Safkan Unified Context Picker Popover
 * Replaces the tab-only picker with support for bookmarks, files, and more.
 */
export const ContextPickerPopover: FC<ContextPickerPopoverProps> = (props) => {
  if (props.variant === 'mention') {
    return <ContextPickerMentionPopover {...props} />
  }
  return <ContextPickerSelectorPopover {...props} />
}

const ContextPickerMentionPopover: FC<ContextPickerMentionPopoverProps> = ({
  isOpen,
  filterText,
  selectedItems,
  onToggleItem,
  onClose,
  anchorRef,
  side,
}) => {
  const { items, isLoading } = useContextSources({
    enabled: isOpen,
    filterText,
  })
  
  const selectedItemIds = useMemo(
    () => new Set(selectedItems.map((t) => t.id)),
    [selectedItems],
  )
  const [focusedIndex, setFocusedIndex] = useState(0)
  const listRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    setFocusedIndex(0)
  }, [filterText])

  useEffect(() => {
    if (!isOpen) return

    const handleKeyDown = (e: KeyboardEvent) => {
      const isNavKey =
        e.key === 'ArrowDown' ||
        e.key === 'ArrowUp' ||
        e.key === 'Enter' ||
        e.key === 'Escape' ||
        e.key === 'Tab'

      if (isNavKey) {
        e.stopPropagation()
      }

      switch (e.key) {
        case 'ArrowDown':
          e.preventDefault()
          setFocusedIndex((prev) => (prev < items.length - 1 ? prev + 1 : prev))
          break
        case 'ArrowUp':
          e.preventDefault()
          setFocusedIndex((prev) => (prev > 0 ? prev - 1 : prev))
          break
        case 'Enter':
          e.preventDefault()
          if (items[focusedIndex]) {
            onToggleItem(items[focusedIndex])
          }
          break
        case 'Escape':
          e.preventDefault()
          onClose()
          break
        case 'Tab':
          e.preventDefault()
          onClose()
          break
      }
    }

    document.addEventListener('keydown', handleKeyDown, true)
    return () => document.removeEventListener('keydown', handleKeyDown, true)
  }, [isOpen, items, focusedIndex, onToggleItem, onClose])

  useEffect(() => {
    if (listRef.current && focusedIndex >= 0) {
      const elements = listRef.current.querySelectorAll('[data-context-item]')
      const element = elements[focusedIndex] as HTMLElement
      if (element) {
        element.scrollIntoView({ block: 'nearest' })
      }
    }
  }, [focusedIndex])

  if (!isOpen) return null

  return (
    <Popover open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <PopoverAnchor virtualRef={anchorRef as React.RefObject<HTMLElement>} />
      <PopoverContent
        side={side ?? 'top'}
        align="start"
        sideOffset={8}
        className="w-[calc(100vw-24px)] max-w-[400px] p-0"
        onOpenAutoFocus={(e) => e.preventDefault()}
        onCloseAutoFocus={(e) => e.preventDefault()}
        role="dialog"
        aria-label="Select context to attach"
      >
        <Command
          className="[&_svg:not([class*='text-'])]:text-muted-foreground"
          shouldFilter={false}
        >
          <div className="border-border/50 border-b px-3 py-2">
            <div className="flex items-center justify-between">
              <span className="font-semibold text-muted-foreground text-xs uppercase tracking-wide">
                Attach Context (@)
              </span>
              <span className="text-muted-foreground text-xs">
                {filterText ? `Filtering: "${filterText}"` : 'Tabs & Bookmarks'}
              </span>
            </div>
            {selectedItems.length > 0 && (
              <span className="mt-1 block text-[var(--accent-orange)] text-xs">
                {selectedItems.length} item{selectedItems.length !== 1 ? 's' : ''}{' '}
                selected
              </span>
            )}
          </div>
          <CommandList
            ref={listRef}
            className="max-h-80 overflow-auto"
            role="listbox"
            aria-multiselectable="true"
          >
            <CommandEmpty className="py-6 text-center">
              {isLoading ? (
                <div className="text-muted-foreground text-sm">
                  Searching...
                </div>
              ) : (
                <div className="text-muted-foreground text-sm">
                  No results found for "{filterText}"
                </div>
              )}
            </CommandEmpty>
            <CommandGroup>
              {items.map((item, index) => (
                <CommandItem
                  key={item.id}
                  data-context-item
                  value={item.id}
                  onSelect={() => onToggleItem(item)}
                  onMouseEnter={() => setFocusedIndex(index)}
                  className="p-0 data-[selected=true]:bg-transparent"
                >
                  <ContextListItem
                    item={item}
                    isSelected={selectedItemIds.has(item.id)}
                    className={index === focusedIndex ? 'bg-accent' : undefined}
                  />
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

const ContextPickerSelectorPopover: FC<ContextPickerSelectorPopoverProps> = ({
  children,
  selectedItems,
  onToggleItem,
  side,
}) => {
  const [open, setOpen] = useState(false)
  const [filterText, setFilterText] = useState('')
  const { items, isLoading } = useContextSources({
    enabled: open,
    filterText,
  })

  const selectedItemIds = useMemo(
    () => new Set(selectedItems.map((t) => t.id)),
    [selectedItems],
  )

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent
        side={side ?? 'bottom'}
        align="start"
        className="w-80 p-0"
        role="dialog"
      >
        <Command
          className="[&_svg:not([class*='text-'])]:text-muted-foreground"
          shouldFilter={false}
        >
          <CommandInput
            placeholder="Search tabs, bookmarks..."
            className="h-9"
            value={filterText}
            onValueChange={setFilterText}
          />
          <CommandList className="max-h-80 overflow-auto">
            <div className="border-border/50 border-b px-3 py-2">
              <div className="flex items-center justify-between">
                <span className="font-semibold text-muted-foreground text-xs uppercase tracking-wide">
                  Context
                </span>
                {selectedItems.length > 0 && (
                  <span className="text-[var(--accent-orange)] text-xs">
                    {selectedItems.length} selected
                  </span>
                )}
              </div>
            </div>

            <CommandEmpty className="py-6 text-center">
              {isLoading ? (
                <div className="text-muted-foreground text-sm">Searching...</div>
              ) : (
                <div className="text-muted-foreground text-sm">No results found</div>
              )}
            </CommandEmpty>
            <CommandGroup>
              {items.map((item) => (
                <CommandItem
                  key={item.id}
                  value={item.id}
                  onSelect={() => onToggleItem(item)}
                  className="p-0"
                >
                  <ContextListItem
                    item={item}
                    isSelected={selectedItemIds.has(item.id)}
                    className="p-3"
                  />
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
