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
import type { ContextAttachment } from '@/lib/context-attachments'
import { ContextListItem } from './context-list-item'
import { TabListItem } from './tab-list-item'
import {
  type ContextPickerItem,
  useAvailableContext,
} from './use-available-context'
import { useAvailableTabs } from './use-available-tabs'

type PopoverSide = 'top' | 'bottom' | 'left' | 'right'

interface TabPickerCommonProps {
  selectedTabs: chrome.tabs.Tab[]
  onToggleTab: (tab: chrome.tabs.Tab) => void
}

interface TabPickerMentionPopoverProps extends TabPickerCommonProps {
  variant: 'mention'
  isOpen: boolean
  filterText: string
  selectedContexts?: ContextAttachment[]
  onToggleContext?: (attachment: ContextAttachment) => void
  onClose: () => void
  anchorRef: React.RefObject<HTMLElement | null>
  side?: PopoverSide
}

interface TabPickerSelectorPopoverProps
  extends PropsWithChildren<TabPickerCommonProps> {
  variant: 'selector'
  side?: PopoverSide
}

type TabPickerPopoverProps =
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
  selectedContexts = [],
  onToggleTab,
  onToggleContext,
  onClose,
  anchorRef,
  side,
}) => {
  const contextEnabled = Boolean(onToggleContext)
  const { tabs, files, memories, allTabs, isLoading, hasWorkspace } =
    useAvailableContext({
      enabled: isOpen,
      filterText,
      includeAttachments: contextEnabled,
    })
  const visibleFiles = contextEnabled ? files : []
  const visibleMemories = contextEnabled ? memories : []
  const items = useMemo<ContextPickerItem[]>(
    () => [
      ...tabs.map((tab) => ({ type: 'tab' as const, tab })),
      ...visibleFiles.map((attachment) => ({
        type: 'file' as const,
        attachment,
      })),
      ...visibleMemories.map((attachment) => ({
        type: 'memory' as const,
        attachment,
      })),
    ],
    [tabs, visibleFiles, visibleMemories],
  )
  const selectedTabIds = useMemo(
    () => new Set(selectedTabs.map((t) => t.id)),
    [selectedTabs],
  )
  const selectedContextIds = useMemo(
    () => new Set(selectedContexts.map((context) => context.id)),
    [selectedContexts],
  )
  const [focusedIndex, setFocusedIndex] = useState(0)
  const listRef = useRef<HTMLDivElement>(null)
  const selectedCount = selectedTabs.length + selectedContexts.length

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
          setFocusedIndex((prev) => (prev < items.length - 1 ? prev + 1 : prev))
          break
        case 'ArrowUp':
          e.preventDefault()
          setFocusedIndex((prev) => (prev > 0 ? prev - 1 : prev))
          break
        case 'Enter':
          e.preventDefault()
          toggleContextPickerItem(items[focusedIndex], {
            onToggleTab,
            onToggleContext,
          })
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
  }, [isOpen, items, focusedIndex, onToggleTab, onToggleContext, onClose])

  useEffect(() => {
    if (listRef.current && focusedIndex >= 0) {
      const elements = listRef.current.querySelectorAll('[data-context-item]')
      elements[focusedIndex]?.scrollIntoView({ block: 'nearest' })
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
                Attach Context
              </span>
              <span className="text-muted-foreground text-xs">
                {filterText ? `Filtering: "${filterText}"` : 'Type to filter'}
              </span>
            </div>
            {selectedCount > 0 && (
              <span className="mt-1 block text-[var(--accent-orange)] text-xs">
                {selectedCount} item{selectedCount !== 1 ? 's' : ''} selected
              </span>
            )}
          </div>
          <CommandList
            ref={listRef}
            className="max-h-64 overflow-auto"
            role="listbox"
            aria-label="Available context"
            aria-multiselectable="true"
          >
            <CommandEmpty className="py-6 text-center">
              {isLoading ? (
                <div className="text-muted-foreground text-sm">
                  Loading context...
                </div>
              ) : (
                <>
                  <div className="text-muted-foreground text-sm">
                    {getContextEmptyTitle({
                      allTabsCount: allTabs.length,
                      hasWorkspace,
                      filterText,
                      contextEnabled,
                    })}
                  </div>
                  <div className="mt-1 text-muted-foreground/70 text-xs">
                    {getContextEmptyDescription({
                      allTabsCount: allTabs.length,
                      hasWorkspace,
                      contextEnabled,
                    })}
                  </div>
                </>
              )}
            </CommandEmpty>
            {tabs.length > 0 ? (
              <CommandGroup heading="Tabs">
                {tabs.map((tab, index) => (
                  <CommandItem
                    key={`tab:${tab.id}`}
                    data-context-item
                    value={`${tab.id}`}
                    onSelect={() => onToggleTab(tab)}
                    onMouseEnter={() => setFocusedIndex(index)}
                    className="p-0 data-[selected=true]:bg-transparent"
                  >
                    <TabListItem
                      tab={tab}
                      isSelected={selectedTabIds.has(tab.id)}
                      className={
                        index === focusedIndex ? 'bg-accent' : undefined
                      }
                    />
                  </CommandItem>
                ))}
              </CommandGroup>
            ) : null}
            {visibleFiles.length > 0 ? (
              <CommandGroup heading="Files">
                {visibleFiles.map((attachment, index) => {
                  const itemIndex = tabs.length + index
                  return (
                    <CommandItem
                      key={attachment.id}
                      data-context-item
                      value={attachment.id}
                      onSelect={() => onToggleContext?.(attachment)}
                      onMouseEnter={() => setFocusedIndex(itemIndex)}
                      className="p-0 data-[selected=true]:bg-transparent"
                    >
                      <ContextListItem
                        attachment={attachment}
                        isSelected={selectedContextIds.has(attachment.id)}
                        className={
                          itemIndex === focusedIndex ? 'bg-accent' : undefined
                        }
                      />
                    </CommandItem>
                  )
                })}
              </CommandGroup>
            ) : null}
            {visibleMemories.length > 0 ? (
              <CommandGroup heading="Memories">
                {visibleMemories.map((attachment, index) => {
                  const itemIndex = tabs.length + visibleFiles.length + index
                  return (
                    <CommandItem
                      key={attachment.id}
                      data-context-item
                      value={attachment.id}
                      onSelect={() => onToggleContext?.(attachment)}
                      onMouseEnter={() => setFocusedIndex(itemIndex)}
                      className="p-0 data-[selected=true]:bg-transparent"
                    >
                      <ContextListItem
                        attachment={attachment}
                        isSelected={selectedContextIds.has(attachment.id)}
                        className={
                          itemIndex === focusedIndex ? 'bg-accent' : undefined
                        }
                      />
                    </CommandItem>
                  )
                })}
              </CommandGroup>
            ) : null}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

function toggleContextPickerItem(
  item: ContextPickerItem | undefined,
  handlers: {
    onToggleTab: (tab: chrome.tabs.Tab) => void
    onToggleContext?: (attachment: ContextAttachment) => void
  },
) {
  if (!item) return
  if (item.type === 'tab') {
    handlers.onToggleTab(item.tab)
    return
  }
  handlers.onToggleContext?.(item.attachment)
}

function getContextEmptyTitle({
  allTabsCount,
  hasWorkspace,
  filterText,
  contextEnabled,
}: {
  allTabsCount: number
  hasWorkspace: boolean
  filterText: string
  contextEnabled: boolean
}): string {
  if (filterText) return `No context matching "${filterText}"`
  if (!contextEnabled) return allTabsCount === 0 ? 'No active tabs' : 'No tabs'
  if (!hasWorkspace && allTabsCount === 0) return 'No context available'
  return 'No context found'
}

function getContextEmptyDescription({
  allTabsCount,
  hasWorkspace,
  contextEnabled,
}: {
  allTabsCount: number
  hasWorkspace: boolean
  contextEnabled: boolean
}): string {
  if (!contextEnabled) {
    return allTabsCount === 0
      ? 'Open some web pages to attach them'
      : 'Try a different search term'
  }
  if (!hasWorkspace) return 'Select a workspace to attach files'
  return 'Try a different search term'
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
