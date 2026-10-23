/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { ChevronDown } from 'lucide-react'
import { useEffect, useId, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { BrowserOSImportSource } from '../browseros-onboarding-api'
import type { ChatAction, DockItem } from '../onboarding-chat'

export interface ReplyDockProps {
  items: readonly DockItem[]
  visible: boolean
  /** Runs inside the click handler: `import` must reach Chromium on this stack. */
  onAction: (action: ChatAction, label: string) => void
  onPickSource: (sourceId: string) => void
}

/** The only place the person acts: quiet replies left, the real choice right. */
export function ReplyDock({
  items,
  visible,
  onAction,
  onPickSource,
}: ReplyDockProps) {
  const shown = visible ? items : []
  const quiet = shown.filter((item): item is ReplyItem => item.kind === 'quiet')
  const main = shown.filter((item) => item.kind !== 'quiet')
  return (
    <div className="relative z-20 mt-5 flex h-[60px] shrink-0 items-center justify-end gap-7">
      {quiet.map((item) => (
        <DockButton key={item.label} item={item} onAction={onAction} />
      ))}
      {main.length > 0 && (
        <div className="flex animate-fade-up items-center gap-2.5">
          {main.map((item) =>
            item.kind === 'profile' ? (
              <ProfileChip
                key="profile"
                sources={item.sources}
                selectedId={item.selectedId}
                onPick={onPickSource}
              />
            ) : (
              <DockButton key={item.label} item={item} onAction={onAction} />
            ),
          )}
        </div>
      )}
    </div>
  )
}

type ReplyItem = Exclude<DockItem, { kind: 'profile' }>

function DockButton({
  item,
  onAction,
}: {
  item: ReplyItem
  onAction: ReplyDockProps['onAction']
}) {
  if (item.kind === 'quiet') {
    return (
      <button
        type="button"
        onClick={() => onAction(item.action, item.label)}
        className="animate-fade-up cursor-pointer whitespace-nowrap rounded-md px-0.5 py-1.5 font-semibold text-[17px] text-ink-2 leading-[22px] outline-none transition-colors hover:text-ink-deep focus-visible:ring-4 focus-visible:ring-primary/25"
      >
        {item.prefix && (
          <span className="font-medium text-ink-3">{item.prefix} </span>
        )}
        {item.label}
      </button>
    )
  }
  return (
    <Button
      type="button"
      variant={item.kind === 'primary' ? 'default' : 'outline'}
      disabled={item.disabled}
      onClick={() => onAction(item.action, item.label)}
      className={cn(
        'h-[60px] rounded-full px-8 font-semibold text-[19px] leading-[22px] focus-visible:ring-4 focus-visible:ring-primary/25 disabled:opacity-40',
        item.kind === 'primary'
          ? 'hover:bg-accent-ink'
          : 'border-border-strong bg-card px-7 text-ink-deep hover:bg-card-tint',
      )}
    >
      {item.label}
    </Button>
  )
}

const AVATAR_TONES = [
  'bg-accent-tint-2 text-ink-deep',
  'bg-ink-deep text-white',
  'bg-secondary text-primary ring-1 ring-accent-tint-2 ring-inset',
]

function SourceAvatar({
  source,
  index,
  className,
}: {
  source: BrowserOSImportSource
  index: number
  className?: string
}) {
  const letter = (source.profileName || source.browserName || '?')
    .charAt(0)
    .toUpperCase()
  return (
    <span
      aria-hidden
      className={cn(
        'flex shrink-0 items-center justify-center rounded-full font-bold',
        AVATAR_TONES[index % AVATAR_TONES.length],
        className,
      )}
    >
      {letter}
    </span>
  )
}

interface ProfileChipProps {
  sources: readonly BrowserOSImportSource[]
  selectedId: string
  onPick: (sourceId: string) => void
}

/**
 * Names the source by Chromium's `displayName` ("Google Chrome - Work"). One
 * profile is the common case: then the chip is a plain label with no chevron.
 */
function ProfileChip({ sources, selectedId, onPick }: ProfileChipProps) {
  const [open, setOpen] = useState(false)
  const wrapRef = useRef<HTMLDivElement>(null)
  const listId = useId()
  const selectedIndex = Math.max(
    0,
    sources.findIndex((source) => source.id === selectedId),
  )
  const selected = sources[selectedIndex]
  const single = sources.length <= 1

  useEffect(() => {
    if (!open) return
    const close = (event: Event) => {
      if (
        event instanceof KeyboardEvent
          ? event.key === 'Escape'
          : !wrapRef.current?.contains(event.target as Node)
      ) {
        setOpen(false)
      }
    }
    document.addEventListener('pointerdown', close)
    document.addEventListener('keydown', close)
    return () => {
      document.removeEventListener('pointerdown', close)
      document.removeEventListener('keydown', close)
    }
  }, [open])

  if (!selected) return null
  const label = (
    <>
      <SourceAvatar
        source={selected}
        index={selectedIndex}
        className="size-9 text-[16px]"
      />
      <span className="max-w-[230px] truncate font-semibold">
        {selected.displayName}
      </span>
    </>
  )
  const chip =
    'flex h-[60px] items-center gap-2.5 whitespace-nowrap rounded-full border border-[#CBD5E1] bg-card pl-3 text-[17px] text-ink-deep leading-[22px]'

  if (single) {
    return (
      <div className={cn(chip, 'pr-[22px]')} title={selected.displayName}>
        {label}
      </div>
    )
  }

  return (
    <div ref={wrapRef} className="relative">
      <button
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        title={selected.displayName}
        onClick={() => setOpen(!open)}
        className={cn(
          chip,
          'cursor-pointer pr-5 outline-none transition-[border-color,box-shadow] hover:border-ink-4 focus-visible:ring-4 focus-visible:ring-primary/25',
          open && 'border-[1.5px] border-primary ring-4 ring-primary/12',
        )}
      >
        {label}
        <ChevronDown
          aria-hidden
          className={cn(
            'size-4 text-ink-2 transition-transform duration-300',
            open && 'rotate-180',
          )}
        />
      </button>
      {open && (
        // Sized to the longest displayName, so names read whole; only past
        // the cap do they ellipsize, with the full name as a tooltip. The
        // height is capped too: the menu opens upward inside an overflow-
        // hidden page, so a long profile list scrolls rather than leaving it.
        <div className="absolute right-0 bottom-[calc(100%+26px)] flex max-h-[min(60vh,360px)] w-max min-w-[300px] max-w-[min(460px,calc(100vw-32px))] animate-fade-up flex-col rounded-[18px] border border-accent-tint-2 bg-card p-2 shadow-[0_30px_60px_-28px_rgb(1_18_63/0.4)]">
          <div className="shrink-0 px-3 pt-2.5 pb-2 font-medium font-mono text-[11px] text-ink-3 uppercase leading-[14px] tracking-[0.12em]">
            Pick a profile
          </div>
          <div
            id={listId}
            role="listbox"
            aria-label="Pick a profile"
            className="min-h-0 overflow-y-auto"
          >
            {sources.map((source, index) => {
              const isSelected = source.id === selected.id
              return (
                <button
                  key={source.id}
                  type="button"
                  role="option"
                  aria-selected={isSelected}
                  title={source.displayName}
                  onClick={() => {
                    onPick(source.id)
                    setOpen(false)
                  }}
                  className={cn(
                    'flex h-14 w-full cursor-pointer items-center gap-3 rounded-xl px-3 text-left text-[17px] text-ink-deep leading-[22px] outline-none hover:bg-card-tint focus-visible:ring-2 focus-visible:ring-primary/40',
                    isSelected ? 'bg-secondary font-semibold' : 'font-medium',
                  )}
                >
                  <SourceAvatar
                    source={source}
                    index={index}
                    className="size-8 text-[15px]"
                  />
                  <span className="min-w-0 flex-1 truncate">
                    {source.displayName}
                  </span>
                  <span
                    aria-hidden
                    className={cn(
                      'flex size-5 shrink-0 items-center justify-center rounded-full border-2',
                      isSelected ? 'border-primary' : 'border-[#CBD5E1]',
                    )}
                  >
                    {isSelected && (
                      <span className="size-2.5 rounded-full bg-primary" />
                    )}
                  </span>
                </button>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}
