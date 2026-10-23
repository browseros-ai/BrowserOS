/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Check, LoaderCircle, Plus } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { cn } from '@/lib/utils'
import type { BrowserOSImportItem } from '../browseros-onboarding-api'
import type { ChatBubble, ChatTurn } from '../onboarding-chat'
import type { ChatReveal } from '../use-chat-reveal'
import { FlightScene } from './FlightScene'
import { MacKeychainPreview } from './MacKeychainPreview'
import { NeoMark } from './NeoMark'

export interface ChatThreadProps {
  turns: readonly ChatTurn[]
  reveal: ChatReveal
  onToggleItem: (item: BrowserOSImportItem) => void
}

/**
 * The scrolling transcript. It always follows the newest line, and once older
 * lines scroll away a fade at the top keeps them readable as history.
 */
export function ChatThread({ turns, reveal, onToggleItem }: ChatThreadProps) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const trackRef = useRef<HTMLDivElement>(null)
  const [scrolled, setScrolled] = useState(false)

  useEffect(() => {
    const scroller = scrollRef.current
    const track = trackRef.current
    if (!scroller || !track) return
    const followEnd = () => {
      const reduced = window.matchMedia(
        '(prefers-reduced-motion: reduce)',
      ).matches
      scroller.scrollTo({
        top: scroller.scrollHeight,
        behavior: reduced ? 'auto' : 'smooth',
      })
    }
    const observer = new ResizeObserver(followEnd)
    observer.observe(track)
    observer.observe(scroller)
    return () => observer.disconnect()
  }, [])

  return (
    <div className="relative min-h-0 flex-1">
      <div
        ref={scrollRef}
        onScroll={(event) => setScrolled(event.currentTarget.scrollTop > 2)}
        className="absolute inset-y-0 -right-3.5 -left-3.5 overflow-y-auto overflow-x-hidden px-3.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        <div
          ref={trackRef}
          className="flex flex-col gap-[18px] pt-1 pb-3"
          aria-live="polite"
        >
          {turns.map((turn, index) => (
            <Turn
              key={turn.id}
              turn={turn}
              first={index === 0}
              reveal={reveal}
              onToggleItem={onToggleItem}
            />
          ))}
        </div>
      </div>
      <div
        aria-hidden
        className={cn(
          'pointer-events-none absolute -inset-x-5 -top-2 z-10 h-24 bg-linear-to-b from-35% from-bg-canvas to-bg-canvas/0 transition-opacity duration-400',
          scrolled ? 'opacity-100' : 'opacity-0',
        )}
      />
    </div>
  )
}

interface TurnProps {
  turn: ChatTurn
  first: boolean
  reveal: ChatReveal
  onToggleItem: (item: BrowserOSImportItem) => void
}

function Turn({ turn, first, reveal, onToggleItem }: TurnProps) {
  const bubbles = turn.bubbles.filter((bubble) =>
    reveal.revealed.has(bubble.id),
  )
  const typing = reveal.typingTurnId === turn.id

  if (turn.from === 'you') {
    if (bubbles.length === 0) return null
    return (
      <div className="flex shrink-0 animate-fade-up justify-end">
        {bubbles.map((bubble) => (
          <div
            key={bubble.id}
            className="rounded-[20px_20px_6px_20px] bg-primary px-5 py-3 font-medium text-[19px] text-primary-foreground leading-[27px]"
          >
            {bubble.kind === 'text' ? bubble.text : null}
          </div>
        ))}
      </div>
    )
  }

  if (bubbles.length === 0 && !typing) return null
  return (
    <div className={cn('flex shrink-0 items-start', first ? 'gap-4' : 'gap-3')}>
      <NeoMark
        className={cn(
          'origin-[50%_60%] transition-transform duration-400 ease-out motion-reduce:transition-none',
          first ? 'size-20' : 'size-10',
          typing && '-rotate-8',
        )}
      />
      <div className="flex min-w-0 flex-col items-start gap-1.5">
        {bubbles.map((bubble, index) => (
          <div
            key={bubble.id}
            className="max-w-full animate-fade-up motion-reduce:animate-none"
          >
            <Bubble
              bubble={bubble}
              lead={index === 0}
              onToggleItem={onToggleItem}
            />
          </div>
        ))}
        {typing && (
          <output
            aria-label="neo is typing"
            className="flex animate-fade-up gap-1.5 rounded-[20px] bg-secondary px-5 py-[21px] first:rounded-tl-[6px]"
          >
            {[0, 200, 400].map((delay) => (
              <i
                key={delay}
                style={{ animationDelay: `${delay}ms` }}
                className="size-2 animate-typing rounded-full bg-ink-3 motion-reduce:animate-none"
              />
            ))}
          </output>
        )}
      </div>
    </div>
  )
}

const BUBBLE =
  'rounded-[20px] bg-secondary px-[18px] py-3 text-[19px] text-ink-deep leading-[27px]'

interface BubbleProps {
  bubble: ChatBubble
  /** The first bubble in a turn points at neo with a tighter corner. */
  lead: boolean
  onToggleItem: (item: BrowserOSImportItem) => void
}

function Bubble({ bubble, lead, onToggleItem }: BubbleProps) {
  const corner = lead && 'rounded-tl-[6px]'
  switch (bubble.kind) {
    case 'hero':
      return (
        <h1
          className={cn(
            BUBBLE,
            corner,
            'm-0 px-6 pt-3.5 pb-4 font-extrabold text-[36px] leading-[42px] tracking-[-0.035em]',
          )}
        >
          Hi, I’m BrowserOS{' '}
          <em className="font-medium font-serif text-[38px] tracking-[-0.01em]">
            neo
          </em>
          .
        </h1>
      )
    case 'scene':
      return <FlightScene live={bubble.live} />
    case 'text':
      return (
        <p
          className={cn(
            BUBBLE,
            corner,
            'm-0',
            bubble.tone === 'display' &&
              'px-5 font-bold text-[25px] leading-8 tracking-[-0.02em]',
            bubble.tone === 'quiet' && 'text-[15px] text-ink-3 leading-[22px]',
          )}
        >
          {bubble.text}
        </p>
      )
    case 'chips':
      return (
        <ImportChips
          bubble={bubble}
          className={cn(corner)}
          onToggleItem={onToggleItem}
        />
      )
    case 'keychain':
      return (
        // Paper rounds this bubble on every corner: it carries a picture.
        <div className={cn(BUBBLE, 'flex flex-col gap-3.5 pb-[18px]')}>
          <p className="m-0">
            Your Mac will ask for its password. Click{' '}
            <b className="font-bold">Allow</b>.
          </p>
          {/* Half-size picture of the real prompt, so the sheet macOS shows
              next is recognisable rather than alarming. */}
          {/* zoom, unlike a transform, also halves the layout box. */}
          <MacKeychainPreview className="[zoom:0.5]" />
        </div>
      )
    case 'progress':
      return (
        <div
          role="status"
          className={cn(
            BUBBLE,
            corner,
            'flex w-[380px] max-w-full flex-col gap-3 pb-4',
          )}
        >
          <span>{bubble.text}</span>
          <span className="h-1 overflow-hidden rounded-sm bg-accent-tint-2">
            <span
              className="block h-full rounded-sm bg-primary transition-[width] duration-150 ease-linear"
              style={{ width: `${bubble.value * 100}%` }}
            />
          </span>
        </div>
      )
    case 'working':
      return (
        <p
          role="status"
          className={cn(BUBBLE, corner, 'm-0 flex items-center gap-3')}
        >
          <LoaderCircle
            aria-hidden
            className="size-5 text-primary motion-safe:animate-spin"
          />
          {bubble.text}
        </p>
      )
  }
}

interface ImportChipsProps {
  bubble: Extract<ChatBubble, { kind: 'chips' }>
  className?: string
  onToggleItem: (item: BrowserOSImportItem) => void
}

/** The only form control in the chat. On/off groups never reflow on toggle. */
function ImportChips({ bubble, className, onToggleItem }: ImportChipsProps) {
  const groups = [
    bubble.chips.filter((chip) => !chip.optional),
    bubble.chips.filter((chip) => chip.optional),
  ].filter((group) => group.length > 0)
  return (
    <div
      className={cn(
        BUBBLE,
        className,
        'flex w-[508px] max-w-full flex-col gap-3.5 pt-3.5 pb-[18px]',
      )}
    >
      <p className="m-0">I’ll bring these from {bubble.browser}:</p>
      <div className="flex flex-col gap-2">
        {groups.map((group) => (
          <div key={group[0].item} className="flex flex-wrap gap-2">
            {group.map((chip) => (
              <button
                key={chip.item}
                type="button"
                aria-pressed={chip.on}
                disabled={!bubble.editable}
                onClick={() => onToggleItem(chip.item)}
                className={cn(
                  'inline-flex h-10 items-center gap-[7px] whitespace-nowrap rounded-full border-[1.5px] pr-4 pl-3 font-medium text-[17px] leading-[22px] outline-none transition-colors duration-150 focus-visible:ring-4 focus-visible:ring-primary/25 enabled:cursor-pointer',
                  chip.on
                    ? 'border-ink-deep bg-ink-deep text-white enabled:hover:border-ink-deep/90 enabled:hover:bg-ink-deep/90'
                    : 'border-[#B9C9E6] bg-card text-ink-deep enabled:hover:border-ink-4',
                )}
              >
                {chip.on ? (
                  <Check aria-hidden className="size-4" strokeWidth={2} />
                ) : (
                  <Plus aria-hidden className="size-4" strokeWidth={1.8} />
                )}
                {chip.label}
              </button>
            ))}
          </div>
        ))}
      </div>
    </div>
  )
}
