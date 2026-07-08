import {
  type FC,
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { TabPreviewCard } from './TabPreviewCard'
import type { SwitcherState } from './types'

interface TabSwitcherOverlayProps {
  state: SwitcherState
  onSelectTab: (index: number) => void
  onAdvance: (direction: 1 | -1) => void
  onSelect: () => void
  onDismiss: () => void
  onSearchChange: (query: string) => void
}

const CARD_WIDTH_STRIP = 200
const CARD_HEIGHT_STRIP = 140
const CARD_WIDTH_GRID = 176
const CARD_HEIGHT_GRID = 118
const GAP = 12

function getCardDimensions(layoutMode: 'strip' | 'grid') {
  return layoutMode === 'strip'
    ? { width: CARD_WIDTH_STRIP, height: CARD_HEIGHT_STRIP }
    : { width: CARD_WIDTH_GRID, height: CARD_HEIGHT_GRID }
}

function getColumns(containerWidth: number, cardWidth: number): number {
  const available = containerWidth - GAP
  const cell = cardWidth + GAP
  return Math.max(2, Math.floor(available / cell))
}

function clampIndex(index: number, length: number): number {
  return ((index % length) + length) % length
}

export const TabSwitcherOverlay: FC<TabSwitcherOverlayProps> = ({
  state,
  onSelectTab,
  onAdvance,
  onSelect,
  onDismiss,
  onSearchChange,
}) => {
  const { tabs, selectedIndex, thumbnails, searchQuery, layoutMode } = state
  const totalTabs = tabs.length

  const containerRef = useRef<HTMLDivElement>(null)
  const searchInputRef = useRef<HTMLInputElement>(null)
  const [containerWidth, setContainerWidth] = useState(0)

  const showSearch = totalTabs > 20

  const filteredTabs = useMemo(() => {
    if (!showSearch || !searchQuery.trim()) return tabs
    const q = searchQuery.toLowerCase()
    return tabs.filter(
      (t) =>
        t.title?.toLowerCase().includes(q) || t.url?.toLowerCase().includes(q),
    )
  }, [tabs, searchQuery, showSearch])

  const filteredSelectedIndex = useMemo(() => {
    if (!showSearch || !searchQuery.trim()) return selectedIndex
    const currentTab = tabs[selectedIndex]
    if (!currentTab) return 0
    const idx = filteredTabs.indexOf(currentTab)
    return idx >= 0 ? idx : 0
  }, [tabs, filteredTabs, selectedIndex, showSearch, searchQuery])

  const activeIndex =
    showSearch && searchQuery.trim() ? filteredSelectedIndex : selectedIndex
  const activeTabs = showSearch && searchQuery.trim() ? filteredTabs : tabs

  const { width: cardWidth, height: cardHeight } = getCardDimensions(layoutMode)

  const columns =
    layoutMode === 'strip'
      ? activeTabs.length
      : getColumns(containerWidth, cardWidth)

  const selectedCardRef = useRef<HTMLDivElement>(null)

  const getSpatialIndex = useCallback(
    (current: number, direction: 'up' | 'down' | 'left' | 'right'): number => {
      const col = current % columns
      const row = Math.floor(current / columns)

      switch (direction) {
        case 'left':
          if (col === 0) {
            const target = row * columns - 1
            return clampIndex(
              target >= 0 ? target : activeTabs.length - 1,
              activeTabs.length,
            )
          }
          return clampIndex(current - 1, activeTabs.length)
        case 'right':
          if (col === columns - 1 || current === activeTabs.length - 1) {
            return clampIndex(row * columns, activeTabs.length)
          }
          return clampIndex(current + 1, activeTabs.length)
        case 'up': {
          const target = current - columns
          if (target < 0) {
            return clampIndex(current, activeTabs.length)
          }
          return clampIndex(target, activeTabs.length)
        }
        case 'down': {
          const target = current + columns
          const lastInRow = Math.min(
            row * columns + columns - 1,
            activeTabs.length - 1,
          )
          if (current === lastInRow || target >= activeTabs.length) {
            return clampIndex(col, activeTabs.length)
          }
          return clampIndex(target, activeTabs.length)
        }
      }
    },
    [columns, activeTabs.length],
  )

  const resolveIndex = useCallback(
    (spatial: number) => {
      const realIndex =
        showSearch && searchQuery.trim()
          ? tabs.indexOf(activeTabs[spatial])
          : spatial
      return realIndex >= 0 ? realIndex : spatial
    },
    [tabs, activeTabs, showSearch, searchQuery],
  )

  const handleKeyDown = (e: React.KeyboardEvent) => {
    switch (e.key) {
      case 'Tab':
        e.preventDefault()
        e.stopPropagation()
        onAdvance(e.shiftKey ? -1 : 1)
        break
      case 'ArrowLeft':
        e.preventDefault()
        onSelectTab(resolveIndex(getSpatialIndex(activeIndex, 'left')))
        break
      case 'ArrowRight':
        e.preventDefault()
        onSelectTab(resolveIndex(getSpatialIndex(activeIndex, 'right')))
        break
      case 'ArrowUp':
        e.preventDefault()
        onSelectTab(resolveIndex(getSpatialIndex(activeIndex, 'up')))
        break
      case 'ArrowDown':
        e.preventDefault()
        onSelectTab(resolveIndex(getSpatialIndex(activeIndex, 'down')))
        break
      case 'Enter':
        e.preventDefault()
        onSelect()
        break
      case 'Escape':
        e.preventDefault()
        if (showSearch && searchQuery) {
          onSearchChange('')
        } else {
          onDismiss()
        }
        break
    }
  }

  useLayoutEffect(() => {
    if (layoutMode === 'strip' && selectedCardRef.current) {
      selectedCardRef.current.scrollIntoView({
        inline: 'center',
        behavior: 'smooth',
        block: 'nearest',
      })
    }
  }, [layoutMode])

  useLayoutEffect(() => {
    if (!containerRef.current) return
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        setContainerWidth(entry.contentRect.width)
      }
    })
    observer.observe(containerRef.current)
    return () => observer.disconnect()
  }, [])

  useLayoutEffect(() => {
    if (showSearch && searchInputRef.current) {
      searchInputRef.current.focus()
    }
  }, [showSearch])

  if (totalTabs === 0) return null

  const renderSearchInput = () => {
    if (!showSearch) return null
    return (
      <input
        ref={searchInputRef}
        type="text"
        placeholder="Search tabs by title or URL..."
        value={searchQuery}
        onChange={(e) => onSearchChange(e.target.value)}
        style={{
          width: '100%',
          maxWidth: '480px',
          padding: '10px 16px',
          borderRadius: '8px',
          border: '1px solid rgba(255, 255, 255, 0.15)',
          background: 'rgba(255, 255, 255, 0.08)',
          color: '#fff',
          fontSize: '14px',
          outline: 'none',
          fontFamily: 'inherit',
        }}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.stopPropagation()
            if (searchQuery) {
              onSearchChange('')
            } else {
              onDismiss()
            }
          }
          if (e.key === 'ArrowDown' || e.key === 'ArrowRight') {
            e.preventDefault()
            onSelectTab(resolveIndex(getSpatialIndex(0, 'right')))
          }
        }}
      />
    )
  }

  const renderCards = () => {
    const isStrip = layoutMode === 'strip'

    return (
      <div
        ref={containerRef}
        role="listbox"
        style={{
          display: isStrip ? 'flex' : 'grid',
          flexDirection: isStrip ? 'row' : undefined,
          gridTemplateColumns: isStrip
            ? undefined
            : `repeat(${columns}, ${cardWidth}px)`,
          gap: `${GAP}px`,
          overflowX: isStrip ? 'auto' : 'visible',
          overflowY: isStrip ? 'visible' : 'auto',
          padding: '8px 16px',
          maxWidth: isStrip ? '90vw' : 'min(90vw, 1200px)',
          maxHeight: '70vh',
          scrollbarWidth: 'none',
          justifyContent: isStrip ? 'flex-start' : 'center',
          justifyItems: 'center',
        }}
      >
        {activeTabs.map((tab, index) => {
          const isSelected = index === activeIndex
          return (
            <div
              key={tab.id}
              ref={isSelected ? selectedCardRef : undefined}
              role="option"
              aria-selected={isSelected}
              tabIndex={-1}
              onClick={() => {
                const realIndex =
                  showSearch && searchQuery.trim() ? tabs.indexOf(tab) : index
                if (realIndex >= 0) onSelectTab(realIndex)
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  onSelect()
                }
              }}
            >
              <TabPreviewCard
                tab={tab}
                isSelected={isSelected}
                thumbnail={tab.id ? thumbnails[tab.id] : undefined}
                cardWidth={cardWidth}
                cardHeight={cardHeight}
              />
            </div>
          )
        })}
      </div>
    )
  }

  return (
    <div
      onKeyDown={handleKeyDown}
      role="dialog"
      aria-label="Tab switcher"
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: '16px',
        outline: 'none',
        width: '90vw',
        maxWidth: '1200px',
      }}
    >
      {renderSearchInput()}
      {renderCards()}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '16px',
          fontSize: '12px',
          color: 'rgba(255, 255, 255, 0.5)',
        }}
      >
        <span>
          {filteredTabs.length === tabs.length
            ? `${selectedIndex + 1} / ${totalTabs} tabs`
            : `${activeIndex + 1} / ${filteredTabs.length} (filtered from ${totalTabs})`}
        </span>
        <span
          style={{
            width: '4px',
            height: '4px',
            borderRadius: '50%',
            background: 'rgba(255, 255, 255, 0.3)',
          }}
        />
        <span>Alt+Q/Mouse scroll to navigate</span>
        <span
          style={{
            width: '4px',
            height: '4px',
            borderRadius: '50%',
            background: 'rgba(255, 255, 255, 0.3)',
          }}
        />
        <span>
          Esc to {showSearch && searchQuery ? 'clear search' : 'cancel'}
        </span>
      </div>
    </div>
  )
}
