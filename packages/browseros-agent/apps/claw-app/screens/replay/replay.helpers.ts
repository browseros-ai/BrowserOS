import {
  CheckCircle2,
  Compass,
  Eye,
  type LucideIcon,
  MousePointer,
  Paperclip,
  Send,
  Type,
} from 'lucide-react'
import type {
  ReplayFrame,
  ReplayKind,
  ReplayVerb,
} from '@/modules/api/replay.hooks'

export interface VerbMeta {
  label: string
  Icon: LucideIcon
  /** Tailwind class for non-bookmarked verb icon. */
  iconClass: string
}

export const VERB_META: Record<ReplayVerb, VerbMeta> = {
  navigate: { label: 'Navigate', Icon: Compass, iconClass: 'text-blue' },
  read: { label: 'Read', Icon: Eye, iconClass: 'text-ink-3' },
  click: { label: 'Click', Icon: MousePointer, iconClass: 'text-ink-2' },
  type: { label: 'Type', Icon: Type, iconClass: 'text-ink-2' },
  attach: { label: 'Attach', Icon: Paperclip, iconClass: 'text-ink-2' },
  submit: { label: 'Submit', Icon: Send, iconClass: 'text-accent-ink' },
  done: { label: 'Done', Icon: CheckCircle2, iconClass: 'text-green' },
}

interface KindStyle {
  /** Solid color for scrubber bookmarks and the caption overlay glyph. */
  dotClass: string
  /** Tinted background for the timeline-row icon tile. */
  tileBgClass: string
  /** Foreground colour for the timeline-row icon tile. */
  tileFgClass: string
  /** Tinted background + colour for the trailing note pill. */
  noteClass: string
}

export const KIND_STYLE: Record<ReplayKind, KindStyle> = {
  action: {
    dotClass: 'bg-ink-4',
    tileBgClass: 'bg-bg-sunken',
    tileFgClass: 'text-ink-2',
    noteClass: 'bg-bg-sunken text-ink-3',
  },
  approval: {
    dotClass: 'bg-accent',
    tileBgClass: 'bg-accent-tint',
    tileFgClass: 'text-accent-ink',
    noteClass: 'bg-accent-tint text-accent-ink',
  },
  block: {
    dotClass: 'bg-red',
    tileBgClass: 'bg-red-tint',
    tileFgClass: 'text-red',
    noteClass: 'bg-red-tint text-red',
  },
  done: {
    dotClass: 'bg-green',
    tileBgClass: 'bg-green-tint',
    tileFgClass: 'text-green',
    noteClass: 'bg-green-tint text-green',
  },
}

export function formatTime(seconds: number): string {
  const safe = Math.max(0, seconds)
  const m = Math.floor(safe / 60)
  const s = Math.floor(safe % 60)
  return `${m}:${String(s).padStart(2, '0')}`
}

/** Last frame whose `t` is at or before `time`. Falls back to 0. */
export function frameIndexAt(
  frames: readonly ReplayFrame[],
  time: number,
): number {
  let idx = 0
  for (let i = 0; i < frames.length; i++) {
    if (frames[i].t <= time + 0.001) idx = i
  }
  return idx
}

export const PLAYBACK_SPEEDS: readonly number[] = [1, 2, 4]
