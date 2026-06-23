import {
  Compass,
  Eye,
  type LucideIcon,
  MousePointer,
  Paperclip,
  Send,
  Type,
} from 'lucide-react'
import type { ActionVerb } from '@/modules/api/run.hooks'

export interface VerbMeta {
  label: string
  Icon: LucideIcon
  /** Tailwind class for the verb tile's icon color when not running. */
  iconClass: string
}

export const VERB_META: Record<ActionVerb, VerbMeta> = {
  navigate: { label: 'Navigate', Icon: Compass, iconClass: 'text-blue' },
  read: { label: 'Read', Icon: Eye, iconClass: 'text-ink-3' },
  click: { label: 'Click', Icon: MousePointer, iconClass: 'text-ink-2' },
  type: { label: 'Type', Icon: Type, iconClass: 'text-ink-2' },
  attach: { label: 'Attach', Icon: Paperclip, iconClass: 'text-ink-2' },
  submit: { label: 'Submit', Icon: Send, iconClass: 'text-accent-ink' },
}
