import { useCombobox } from 'downshift'
import {
  ArrowRight,
  ChevronDown,
  Folder,
  Globe,
  Layers,
  PlugZap,
  Search,
  X,
} from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useNavigate } from 'react-router'
import { AppSelector } from '@/components/elements/AppSelector'
import {
  GlowingBorder,
  GlowingElement,
} from '@/components/elements/glowing-border'
import { TabPickerPopover } from '@/components/elements/tab-picker-popover'
import { WorkspaceSelector } from '@/components/elements/workspace-selector'
import { Button } from '@/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { McpServerIcon } from '@/entrypoints/app/connect-mcp/McpServerIcon'
import { useGetUserMCPIntegrations } from '@/entrypoints/app/connect-mcp/useGetUserMCPIntegrations'
import { Feature } from '@/lib/browseros/capabilities'
import { useCapabilities } from '@/lib/browseros/useCapabilities'
import {
  createAITabAction,
  createBrowserOSAction,
} from '@/lib/chat-actions/types'
import {
  NEWTAB_AI_TRIGGERED_EVENT,
  NEWTAB_APPS_OPENED_EVENT,
  NEWTAB_CHAT_STARTED_EVENT,
  NEWTAB_OPENED_EVENT,
  NEWTAB_SEARCH_EXECUTED_EVENT,
  NEWTAB_TAB_REMOVED_EVENT,
  NEWTAB_TAB_TOGGLED_EVENT,
  NEWTAB_TABS_OPENED_EVENT,
  NEWTAB_WORKSPACE_OPENED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { useMcpServers } from '@/lib/mcp/mcpServerStorage'
import { useSyncRemoteIntegrations } from '@/lib/mcp/useSyncRemoteIntegrations'
import { openSidePanelWithSearch } from '@/lib/messaging/sidepanel/openSidepanelWithSearch'
import { track } from '@/lib/metrics/track'
import { cn } from '@/lib/utils'
import { useWorkspace } from '@/lib/workspace/use-workspace'
import { ImportDataHint } from './ImportDataHint'
import type { SuggestionItem } from './lib/suggestions/types'
import {
  getSuggestionLabel,
  useSuggestions,
} from './lib/suggestions/useSuggestions'
import { NewTabBranding } from './NewTabBranding'
import { NewTabTip } from './NewTabTip'
import { ScheduleResults } from './ScheduleResults'
import { SearchSuggestions } from './SearchSuggestions'
import { ShortcutsDialog } from './ShortcutsDialog'
import { SignInHint } from './SignInHint'
import { TopSites } from './TopSites'
import { useActiveHint } from './useActiveHint'

interface MentionState {
  isOpen: boolean
  filterText: string
  startPosition: number
}

/**
 * @public
 */
export const NewTab = () => {
  const activeHint = useActiveHint()
  const navigate = useNavigate()
  const [inputValue, setInputValue] = useState('')
  const [mounted, setMounted] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)
  const tabsDropdownRef = useRef<HTMLDivElement>(null)
  const [selectedTabs, setSelectedTabs] = useState<chrome.tabs.Tab[]>([])
  const [shortcutsDialogOpen, setShortcutsDialogOpen] = useState(false)
  const [mentionState, setMentionState] = useState<MentionState>({
    isOpen: false,
    filterText: '',
    startPosition: 0,
  })
  const { selectedFolder } = useWorkspace()
  const { supports } = useCapabilities()
  const { servers: mcpServers } = useMcpServers()
  const { data: userMCPIntegrations } = useGetUserMCPIntegrations()
  useSyncRemoteIntegrations()

  const connectedManagedServers = mcpServers.filter((s) => {
    if (s.type !== 'managed' || !s.managedServerName) return false
    return userMCPIntegrations?.integrations?.find(
      (i) => i.name === s.managedServerName,
    )?.is_authenticated
  })

  const toggleTab = (tab: chrome.tabs.Tab) => {
    setSelectedTabs((prev) => {
      const isSelected = prev.some((t) => t.id === tab.id)
      track(NEWTAB_TAB_TOGGLED_EVENT, {
        action: isSelected ? 'removed' : 'added',
      })
      if (isSelected) {
        return prev.filter((t) => t.id !== tab.id)
      }
      return [...prev, tab]
    })
  }

  const removeTab = (tabId?: number) => {
    track(NEWTAB_TAB_REMOVED_EVENT)
    setSelectedTabs((prev) => prev.filter((t) => t.id !== tabId))
  }

  const { sections, flatItems, providerConfig } = useSuggestions({
    query: inputValue,
    selectedTabs,
  })
  const searchPlaceholder = `Ask BrowserOS or search ${providerConfig.name}...`

  const {
    isOpen,
    getMenuProps,
    getInputProps,
    highlightedIndex,
    getItemProps,
    reset,
    setInputValue: setComboboxInputValue,
  } = useCombobox<SuggestionItem>({
    items: flatItems,
    itemToString: (item) => (item ? getSuggestionLabel(item) : ''),
    onSelectedItemChange({ selectedItem }) {
      if (selectedItem) {
        runSelectedAction(selectedItem)
      }
    },
    onStateChange: ({
      type,
      inputValue: stateInputValue,
      highlightedIndex,
      selectedItem,
    }) => {
      if (type === useCombobox.stateChangeTypes.InputKeyDownEnter) {
        if (!selectedItem && !highlightedIndex && !stateInputValue) {
          executeDefaultAction()
        }
      }
    },
    onInputValueChange({ inputValue: newValue }) {
      setInputValue(newValue ?? '')
    },
  })

  const inputValueRef = useRef(inputValue)
  const mentionStateRef = useRef(mentionState)

  useEffect(() => {
    inputValueRef.current = inputValue
    mentionStateRef.current = mentionState
  })

  const closeMention = useCallback(() => {
    const state = mentionStateRef.current
    if (!state.isOpen) return

    const currentInput = inputValueRef.current
    const beforeMention = currentInput.slice(0, state.startPosition)
    const afterMention = currentInput.slice(
      state.startPosition + 1 + state.filterText.length,
    )
    const nextInput = beforeMention + afterMention
    inputValueRef.current = nextInput
    setComboboxInputValue(nextInput)

    const nextMentionState = { isOpen: false, filterText: '', startPosition: 0 }
    mentionStateRef.current = nextMentionState
    setMentionState(nextMentionState)

    requestAnimationFrame(() => {
      inputRef.current?.focus()
      const newPosition = beforeMention.length
      inputRef.current?.setSelectionRange(newPosition, newPosition)
    })
  }, [setComboboxInputValue])

  const handleInputChange = (value: string) => {
    const input = inputRef.current
    const cursorPosition = input?.selectionStart ?? value.length
    const state = mentionStateRef.current

    if (state.isOpen) {
      const textAfterAt = value.slice(state.startPosition + 1)
      const spaceIndex = textAfterAt.search(/\s/)
      const filterText =
        spaceIndex === -1 ? textAfterAt : textAfterAt.slice(0, spaceIndex)

      if (
        cursorPosition <= state.startPosition ||
        value[state.startPosition] !== '@'
      ) {
        const nextMentionState = {
          isOpen: false,
          filterText: '',
          startPosition: 0,
        }
        mentionStateRef.current = nextMentionState
        setMentionState(nextMentionState)
      } else {
        const nextMentionState = { ...state, filterText }
        mentionStateRef.current = nextMentionState
        setMentionState(nextMentionState)
      }
    } else {
      const charBeforeCursor = value[cursorPosition - 1]
      const textBeforeAt = value.slice(0, cursorPosition - 1)
      const isAtWordBoundary = /(?:^|[\s\n])$/.test(textBeforeAt)

      if (charBeforeCursor === '@' && isAtWordBoundary) {
        const nextMentionState = {
          isOpen: true,
          filterText: '',
          startPosition: cursorPosition - 1,
        }
        mentionStateRef.current = nextMentionState
        setMentionState(nextMentionState)
      }
    }

    inputValueRef.current = value
  }

  useEffect(() => {
    if (!mentionState.isOpen) return

    const handleClickOutside = (e: MouseEvent) => {
      const target = e.target as HTMLElement
      if (inputRef.current?.contains(target)) return
      if (target.closest('[data-slot="popover-content"]')) return
      closeMention()
    }

    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [mentionState.isOpen, closeMention])

  const handleSend = () => {
    if (mentionStateRef.current.isOpen) {
      closeMention()
      return
    }
    if (highlightedIndex > -1) {
      const selectedItem = flatItems[highlightedIndex]
      runSelectedAction(selectedItem)
    } else {
      executeDefaultAction()
    }
  }

  const executeDefaultAction = () => {
    const selectedItem = flatItems[0]
    runSelectedAction(selectedItem)
  }

  const startInlineChat = (
    message: string,
    chatMode: 'chat' | 'agent',
    aiTab?: { name: string; description: string },
  ) => {
    track(NEWTAB_CHAT_STARTED_EVENT, {
      mode: chatMode,
      tabs_count: selectedTabs.length,
    })
    const tabIds = selectedTabs
      .map((t) => t.id)
      .filter((id): id is number => id !== undefined)
    reset()
    setSelectedTabs([])
    const params = new URLSearchParams({ q: message, mode: chatMode })
    if (tabIds.length > 0) {
      params.set('tabs', tabIds.join(','))
    }
    if (aiTab) {
      params.set('actionType', 'ai-tab')
      params.set('tabName', aiTab.name)
      params.set('tabDescription', aiTab.description)
    }
    navigate(`/home/chat?${params.toString()}`)
  }

  const runSelectedAction = (item: SuggestionItem | undefined) => {
    if (!item) return

    switch (item.type) {
      case 'search':
        track(NEWTAB_SEARCH_EXECUTED_EVENT, {
          search_engine: providerConfig.id,
        })
        window.open(
          `${providerConfig.searchUrl}${encodeURIComponent(item.query)}`,
          '_self',
        )
        break
      case 'ai-tab': {
        track(NEWTAB_AI_TRIGGERED_EVENT, {
          mode: 'agent',
          tabs_count: selectedTabs.length,
        })
        const searchQuery = `${item.name}${item.description ? ` - ${item.description}` : ''}}`
        if (supports(Feature.NEWTAB_CHAT_SUPPORT)) {
          startInlineChat(searchQuery, 'agent', {
            name: item.name,
            description: item.description,
          })
        } else {
          const action = createAITabAction({
            name: item.name,
            description: item.description,
            tabs: selectedTabs,
          })
          openSidePanelWithSearch('open', {
            query: searchQuery,
            mode: 'agent',
            action,
          })
          reset()
          setSelectedTabs([])
        }
        break
      }
      case 'browseros': {
        track(NEWTAB_AI_TRIGGERED_EVENT, {
          mode: item.mode,
          tabs_count: selectedTabs.length,
        })
        if (supports(Feature.NEWTAB_CHAT_SUPPORT)) {
          startInlineChat(item.message, item.mode)
        } else {
          const action = createBrowserOSAction({
            mode: item.mode,
            message: item.message,
            tabs: selectedTabs,
          })
          openSidePanelWithSearch('open', {
            query: item.message,
            mode: item.mode,
            action,
          })
          reset()
          setSelectedTabs([])
        }
        break
      }
    }
  }

  const isSuggestionsVisible =
    !mentionState.isOpen &&
    ((isOpen && inputValue.length) ||
      (sections.length > 0 && inputValue.length) ||
      (isOpen && selectedTabs.length))

  useEffect(() => {
    setMounted(true)
    track(NEWTAB_OPENED_EVENT)
  }, [])

  return (
    <div className="pt-[max(25vh,16px)]">
      {/* Main content */}
      <div className={'relative w-full space-y-8 md:w-3xl'}>
        {/* Logo and branding */}
        <NewTabBranding />
        {/* Search bar with context */}
        <div
          className={cn(
            'relative overflow-hidden bg-border/50 p-[2px]',
            isSuggestionsVisible ||
              mentionState.isOpen ||
              selectedTabs.length > 0
              ? 'bg-[var(--accent-orange)]/30 shadow-[var(--accent-orange)]/10'
              : 'bg-border/50 hover:border-border',
          )}
          style={{ borderRadius: '1.5rem' }}
        >
          {mounted && (
            <div
              className="absolute inset-0"
              style={{ borderRadius: '1.5rem' }}
            >
              <GlowingBorder duration={2000} delay={0} rx="1.5rem" ry="1.5rem">
                <GlowingElement />
              </GlowingBorder>
            </div>
          )}
          <div
            className={cn(
              'relative bg-card shadow-lg',
              isSuggestionsVisible ||
                mentionState.isOpen ||
                selectedTabs.length > 0
                ? 'border-[var(--accent-orange)]/30 shadow-[var(--accent-orange)]/10'
                : 'border-border/50 hover:border-border',
            )}
            style={{ borderRadius: 'calc(1.5rem - 2px)' }}
          >
            {/* Main search input */}
            <div className="flex items-center gap-3 px-5 py-4">
              <Search className="h-5 w-5 flex-shrink-0 text-muted-foreground" />

              <TabPickerPopover
                variant="mention"
                isOpen={mentionState.isOpen}
                filterText={mentionState.filterText}
                selectedTabs={selectedTabs}
                onToggleTab={toggleTab}
                onClose={closeMention}
                anchorRef={inputRef}
                side="bottom"
              />
              <input
                type="text"
                placeholder={searchPlaceholder}
                className="flex-1 border-none bg-transparent text-base text-foreground outline-none placeholder:text-muted-foreground"
                {...getInputProps({
                  ref: inputRef,
                  onChange: (e) => handleInputChange(e.currentTarget.value),
                  onKeyDown: (e) => {
                    if (!mentionStateRef.current.isOpen) return
                    if (e.key === 'Tab') {
                      e.preventDefault()
                      closeMention()
                    }
                  },
                })}
              />

              <Button
                onClick={handleSend}
                size="icon"
                className="h-10 w-10 flex-shrink-0 rounded-xl bg-primary text-primary-foreground hover:bg-primary/90"
              >
                <ArrowRight className="h-5 w-5" />
              </Button>
            </div>

            <AnimatePresence>
              {selectedTabs.length > 0 && (
                <motion.div
                  className="overflow-clip px-5 pb-4"
                  transition={{ duration: 0.2 }}
                  initial={{ opacity: 0, height: 0 }}
                  animate={{ opacity: 1, height: 'auto' }}
                  exit={{ opacity: 0, height: 0 }}
                >
                  <div className="styled-scrollbar flex gap-3 overflow-x-auto pb-2">
                    <AnimatePresence>
                      {selectedTabs.map((selectedTab) => {
                        if (!selectedTab) return null
                        return (
                          <motion.div
                            key={selectedTab.id}
                            className="group w-48 flex-shrink-0 overflow-clip rounded-lg border border-border bg-accent/50 p-3 transition-colors hover:bg-accent"
                            transition={{ duration: 0.2 }}
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: 'auto' }}
                            exit={{ opacity: 0, height: 0 }}
                          >
                            <div className="flex items-start gap-3">
                              <div className="flex h-10 w-10 flex-shrink-0 items-center justify-center rounded-lg border border-border bg-background">
                                {selectedTab.favIconUrl ? (
                                  <img
                                    src={selectedTab.favIconUrl}
                                    alt={selectedTab.title}
                                    className="h-6 w-6"
                                  />
                                ) : (
                                  <Globe className="h-6 w-6" />
                                )}
                              </div>
                              <div className="min-w-0 flex-1">
                                <div className="mb-1 truncate font-medium text-foreground text-sm">
                                  {selectedTab.title}
                                </div>
                                <div className="text-muted-foreground text-xs">
                                  Tab
                                </div>
                              </div>
                              <button
                                type="button"
                                onClick={() => removeTab(selectedTab.id)}
                                className="cursor-pointer rounded p-1 opacity-0 transition-opacity hover:bg-background group-hover:opacity-100"
                              >
                                <X className="h-3 w-3" />
                              </button>
                            </div>
                          </motion.div>
                        )
                      })}
                    </AnimatePresence>
                  </div>
                </motion.div>
              )}
            </AnimatePresence>

            <AnimatePresence>
              {isSuggestionsVisible && (
                <SearchSuggestions
                  getItemProps={getItemProps}
                  getMenuProps={getMenuProps}
                  highlightedIndex={highlightedIndex}
                  sections={sections}
                />
              )}
            </AnimatePresence>

            {mounted && (
              <div className="flex items-center justify-between border-border/50 border-t px-5 py-3">
                <div className="flex items-center gap-1">
                  {supports(Feature.WORKSPACE_FOLDER_SUPPORT) && (
                    <WorkspaceSelector>
                      <Button
                        variant="ghost"
                        onClick={() => track(NEWTAB_WORKSPACE_OPENED_EVENT)}
                        className={cn(
                          'flex items-center gap-2 rounded-lg px-3 py-1.5 font-medium text-sm transition-all',
                          'bg-transparent text-muted-foreground hover:bg-accent hover:text-accent-foreground',
                          'data-[state=open]:bg-accent',
                        )}
                      >
                        <Folder className="h-4 w-4" />
                        <span>{selectedFolder?.name || 'Add workspace'}</span>
                        <ChevronDown className="h-3 w-3" />
                      </Button>
                    </WorkspaceSelector>
                  )}

                  <div className="relative" ref={tabsDropdownRef}>
                    <TabPickerPopover
                      variant="selector"
                      selectedTabs={selectedTabs}
                      onToggleTab={toggleTab}
                    >
                      <Button
                        onClick={() => track(NEWTAB_TABS_OPENED_EVENT)}
                        className={cn(
                          'flex items-center gap-2 rounded-lg px-3 py-1.5 font-medium text-sm transition-all',
                          selectedTabs.length > 0
                            ? 'bg-[var(--accent-orange)]! text-white shadow-sm'
                            : 'bg-transparent text-muted-foreground hover:bg-accent hover:text-accent-foreground',
                          'data-[state=open]:bg-accent',
                        )}
                      >
                        <Layers className="h-4 w-4" />
                        <span>Tabs</span>
                      </Button>
                    </TabPickerPopover>
                  </div>
                </div>

                {supports(Feature.MANAGED_MCP_SUPPORT) && (
                  <div className="ml-auto flex items-center gap-1.5">
                    {connectedManagedServers.length === 0 && (
                      <span className="flex items-center gap-1 font-semibold text-[var(--accent-orange)] text-sm">
                        New!
                      </span>
                    )}
                    {connectedManagedServers.length === 0 ? (
                      <Tooltip>
                        <AppSelector side="bottom">
                          <TooltipTrigger asChild>
                            <Button
                              variant="ghost"
                              onClick={() =>
                                track(NEWTAB_APPS_OPENED_EVENT, {
                                  has_connected_apps: false,
                                })
                              }
                              className={cn(
                                'flex items-center gap-2 rounded-lg px-3 py-1.5 font-medium text-sm transition-all',
                                'bg-transparent text-muted-foreground hover:bg-accent hover:text-accent-foreground',
                                'data-[state=open]:bg-accent',
                              )}
                            >
                              <PlugZap className="h-4 w-4" />
                              <span>Apps</span>
                              <ChevronDown className="h-3 w-3" />
                            </Button>
                          </TooltipTrigger>
                        </AppSelector>
                        <TooltipContent side="left" className="max-w-56">
                          Apps directly connected will have more accurate and
                          faster responses for your queries!
                        </TooltipContent>
                      </Tooltip>
                    ) : (
                      <AppSelector side="bottom">
                        <Button
                          variant="ghost"
                          onClick={() =>
                            track(NEWTAB_APPS_OPENED_EVENT, {
                              has_connected_apps: true,
                              connected_count: connectedManagedServers.length,
                            })
                          }
                          className={cn(
                            'flex items-center gap-2 rounded-lg px-3 py-1.5 font-medium text-sm transition-all',
                            'bg-transparent text-muted-foreground hover:bg-accent hover:text-accent-foreground',
                            'data-[state=open]:bg-accent',
                          )}
                        >
                          <div className="flex items-center -space-x-1.5">
                            {connectedManagedServers.slice(0, 4).map((s) => (
                              <div
                                key={s.id}
                                className="rounded-full ring-2 ring-card"
                              >
                                <McpServerIcon
                                  serverName={s.managedServerName ?? ''}
                                  size={16}
                                />
                              </div>
                            ))}
                          </div>
                          {connectedManagedServers.length > 4 && (
                            <span className="text-xs">
                              +{connectedManagedServers.length - 4}
                            </span>
                          )}
                          <span>Apps</span>
                          <ChevronDown className="h-3 w-3" />
                        </Button>
                      </AppSelector>
                    )}
                  </div>
                )}
              </div>
            )}
          </div>
        </div>

        {mounted && !isSuggestionsVisible && <NewTabTip />}

        {/* Top sites */}
        {!isSuggestionsVisible && <TopSites />}

        {mounted && !isSuggestionsVisible && <ScheduleResults />}
      </div>
      {mounted && (
        <ShortcutsDialog
          open={shortcutsDialogOpen}
          onOpenChange={setShortcutsDialogOpen}
        />
      )}
      {activeHint === 'signin' && <SignInHint />}
      {activeHint === 'import' && <ImportDataHint />}
    </div>
  )
}
