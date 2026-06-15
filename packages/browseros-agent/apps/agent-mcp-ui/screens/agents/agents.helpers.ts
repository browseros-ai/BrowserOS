import {
  Bot,
  Code,
  type LucideIcon,
  MousePointer2,
  Sparkles,
  Terminal,
} from 'lucide-react'
import type {
  AgentProfile,
  AgentProfileStatus,
} from '@/modules/api/agents.hooks'

const HARNESS_ICON: Record<AgentProfile['harness'], LucideIcon> = {
  'Claude Cowork': Sparkles,
  Codex: Code,
  Hermes: Bot,
  OpenClaw: MousePointer2,
  'Gemini CLI': Terminal,
}

export function harnessIconFor(harness: AgentProfile['harness']): LucideIcon {
  return HARNESS_ICON[harness]
}

interface ProfileStatusMeta {
  label: string
  /** Tailwind classes for the Badge variant. */
  className: string
}

const PROFILE_STATUS_META: Record<AgentProfileStatus, ProfileStatusMeta> = {
  configured: {
    label: 'Configured',
    className: 'bg-green-tint text-green border-transparent',
  },
  paused: {
    label: 'Paused',
    className: 'bg-amber-tint text-amber border-transparent',
  },
  disabled: {
    label: 'Disabled',
    className: 'bg-bg-sunken text-ink-3 border-transparent',
  },
}

export function statusMetaFor(status: AgentProfileStatus): ProfileStatusMeta {
  return PROFILE_STATUS_META[status]
}

export function scopeSummaryFor(profile: AgentProfile): string {
  const parts = [
    `${profile.loginCount} login${profile.loginCount === 1 ? '' : 's'}`,
    `${profile.aclRuleCount} ACL rule${profile.aclRuleCount === 1 ? '' : 's'}`,
  ]
  if (profile.blockedActionCount > 0) {
    parts.push(
      `${profile.blockedActionCount} blocked action${profile.blockedActionCount === 1 ? '' : 's'}`,
    )
  }
  if (profile.alwaysAllowCount > 0) {
    parts.push(
      `${profile.alwaysAllowCount} always-allow grant${profile.alwaysAllowCount === 1 ? '' : 's'}`,
    )
  }
  return parts.join(' · ')
}
