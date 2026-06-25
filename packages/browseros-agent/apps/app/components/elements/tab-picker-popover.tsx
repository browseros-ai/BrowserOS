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
import { useAvailableTabs } from './available-tabs.hooks'
import { TabListItem } from './tab-list-item'

export type PopoverSide = 'top' | 'bottom' | 'left' | 'right'

export interface TabPickerCommonProps {
  selectedTabs: chrome.tabs.Tab[]
  onToggleTab: (tab: chrome.tabs.Tab) => void
}

export interface TabPickerMentionPopoverProps extends TabPickerCommonProps {
  variant: 'mention'
  isOpen: boolean
  filterText: string
  onClose: () => void
  anchorRef: React.RefObject<HTMLElement | null>
  side?: PopoverSide
}

export interface TabPickerSelectorPopoverProps
  extends PropsWithChildren<TabPickerCommonProps> {
  variant: 'selector'
  side?: PopoverSide
}

export type TabPickerPopoverProps =
  | TabPickerMentionPopoverProps
  | TabPickerSelectorPopoverProps

export const TabPickerPopover: FC<TabPickerPopoverProps> = (props) => {
  if (props.variant === 'mention') {
    return <TabPickerMentionPopover {...props} />
  }
  return <TabPickerSelectorPopover {...props} />
}

const TabPickerMentionPopover: FC<TabPickerMentionPopoverProps> = ({
  isOpen,
  filterText,
  selectedTabs,
  onToggleTab,
  onClose,
  anchorRef,
  side,
}) => {
  const { tabs, allTabs, isLoading } = useAvailableTabs({
    enabled: isOpen,
    filterText,
  })
  const selectedTabIds = useMemo(
    () => new Set(selectedTabs.map((t) => t.id)),
    [selectedTabs],
  )
  const [focusedIndex, setFocusedIndex] = useState(0)
  const listRef = useRef<HTMLDivElement>(null)

  // biome-ignore lint/correctness/useExhaustiveDependencies: intentionally reset focus when filter changes
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
          setFocusedIndex((prev) => (prev < tabs.length - 1 ? prev + 1 : prev))
          break
        case 'ArrowUp':
          e.preventDefault()
          setFocusedIndex((prev) => (prev > 0 ? prev - 1 : prev))
          break
        case 'Enter':
          e.preventDefault()
          if (tabs[focusedIndex]) {
            onToggleTab(tabs[focusedIndex])
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
  }, [isOpen, tabs, focusedIndex, onToggleTab, onClose])

  useEffect(() => {
    if (listRef.current && focusedIndex >= 0) {
      const items = listRef.current.querySelectorAll('[data-tab-item]')
      items[focusedIndex]?.scrollIntoView({ block: 'nearest' })
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
        aria-label="Select tabs to attach"
      >
        <Command
          className="[&_svg:not([class*='text-'])]:text-muted-foreground"
          shouldFilter={false}
        >
          <div className="border-border/50 border-b px-3 py-2">
            <div className="flex items-center justify-between">
              <span className="font-semibold text-muted-foreground text-xs uppercase tracking-wide">
                Attach Tabs
              </span>
              <span className="text-muted-foreground text-xs">
                {filterText ? `Filtering: "${filterText}"` : 'Type to filter'}
              </span>
            </div>
            {selectedTabs.length > 0 && (
              <span className="mt-1 block text-[var(--accent-orange)] text-xs">
                {selectedTabs.length} tab{selectedTabs.length !== 1 ? 's' : ''}{' '}
                selected
              </span>
            )}
          </div>
          <CommandList
            ref={listRef}
            className="max-h-64 overflow-auto"
            role="listbox"
            aria-label="Available tabs"
            aria-multiselectable="true"
          >
            <CommandEmpty className="py-6 text-center">
              {isLoading ? (
                <div className="text-muted-foreground text-sm">
                  Loading tabs…
                </div>
              ) : (
                <>
                  <div className="text-muted-foreground text-sm">
                    {allTabs.length === 0
                      ? 'No active tabs'
                      : `No tabs matching "${filterText}"`}
                  </div>
                  <div className="mt-1 text-muted-foreground/70 text-xs">
                    {allTabs.length === 0
                      ? 'Open some web pages to attach them'
                      : 'Try a different search term'}
                  </div>
                </>
              )}
            </CommandEmpty>
            <CommandGroup>
              {tabs.map((tab, index) => (
                <CommandItem
                  key={tab.id}
                  data-tab-item
                  value={`${tab.id}`}
                  onSelect={() => onToggleTab(tab)}
                  onMouseEnter={() => setFocusedIndex(index)}
                  className="p-0 data-[selected=true]:bg-transparent"
                >
                  <TabListItem
                    tab={tab}
                    isSelected={selectedTabIds.has(tab.id)}
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

const TabPickerSelectorPopover: FC<TabPickerSelectorPopoverProps> = ({
  children,
  selectedTabs,
  onToggleTab,
  side,
}) => {
  const [open, setOpen] = useState(false)
  const [filterText, setFilterText] = useState('')
  const { tabs, allTabs, isLoading } = useAvailableTabs({
    enabled: open,
    filterText,
  })

  const selectedTabIds = useMemo(
    () => new Set(selectedTabs.map((t) => t.id)),
    [selectedTabs],
  )

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent
        side={side ?? 'bottom'}
        align="start"
        className="w-72 p-0"
        role="dialog"
        aria-label="Select tabs"
      >
        <Command
          className="[&_svg:not([class*='text-'])]:text-muted-foreground"
          shouldFilter={false}
        >
          <CommandInput
            placeholder="Search tabs..."
            className="h-9"
            value={filterText}
            onValueChange={setFilterText}
          />
          <CommandList
            className="max-h-64 overflow-auto"
            role="listbox"
            aria-label="Available tabs"
            aria-multiselectable="true"
          >
            <div className="border-border/50 border-b px-3 py-2">
              <div className="flex items-center justify-between">
                <span className="font-semibold text-muted-foreground text-xs uppercase tracking-wide">
                  Tabs
                </span>
                {selectedTabs.length > 0 && (
                  <span className="text-[var(--accent-orange)] text-xs">
                    {selectedTabs.length} selected
                  </span>
                )}
              </div>
            </div>

            <CommandEmpty className="py-6 text-center">
              {isLoading ? (
                <div className="text-muted-foreground text-sm">
                  Loading tabs…
                </div>
              ) : (
                <>
                  <div className="text-muted-foreground text-sm">
                    {allTabs.length === 0
                      ? 'No active tabs'
                      : `No tabs matching "${filterText}"`}
                  </div>
                  <div className="mt-1 text-muted-foreground/70 text-xs">
                    {allTabs.length === 0
                      ? 'Open some web pages to attach them'
                      : 'Try a different search term'}
                  </div>
                </>
              )}
            </CommandEmpty>
            <CommandGroup>
              {tabs.map((tab) => (
                <CommandItem
                  key={tab.id}
                  value={`${tab.id} ${tab.title} ${tab.url}`}
                  onSelect={() => onToggleTab(tab)}
                  className="p-0"
                >
                  <TabListItem
                    tab={tab}
                    isSelected={selectedTabIds.has(tab.id)}
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
