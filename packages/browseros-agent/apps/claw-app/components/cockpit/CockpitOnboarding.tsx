/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Check } from 'lucide-react'
import { useState } from 'react'
import { NavLink } from 'react-router'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  CONNECTED_COPY,
  FOOTER_COPY,
  HERO_COPY,
  MANAGE_COPY,
  type OnboardingState,
  PANEL_COPY,
  STARTER_PROMPT,
  STARTER_PROMPT_LABEL,
} from '@/screens/cockpit/cockpit-onboarding.helpers'
import { StarterPromptTile } from './StarterPromptTile'
import { VideoFeature } from './VideoFeature'

interface CockpitOnboardingProps {
  state: Exclude<OnboardingState, 'ready'>
  connectedHarnesses?: readonly string[]
}

export function CockpitOnboarding({
  state,
  connectedHarnesses = [],
}: CockpitOnboardingProps) {
  const [promptCopied, setPromptCopied] = useState(false)
  const isWaiting = state === 'waiting'
  const listening = isWaiting || promptCopied
  const statusText = promptCopied
    ? PANEL_COPY.copied
    : isWaiting
      ? PANEL_COPY.waiting
      : PANEL_COPY.tieBack
  const flagCopied = () => {
    track(AnalyticsEvent.OnboardingTaskCopied)
    setPromptCopied(true)
    window.setTimeout(() => setPromptCopied(false), 8000)
  }
  return (
    <section
      className="flex flex-col gap-8"
      aria-label={HERO_COPY.eyebrow.toLowerCase()}
    >
      <OnboardingHero />
      <div className="grid gap-6 md:grid-cols-12 md:items-stretch">
        <div className="md:col-span-7">
          <VideoFeature />
        </div>
        <StartPanel
          className="md:col-span-5"
          harnesses={connectedHarnesses}
          listening={listening}
          statusText={statusText}
          onPromptCopied={flagCopied}
        />
      </div>
      <OnboardingFooter />
    </section>
  )
}

function OnboardingHero() {
  return (
    <header className="flex flex-col gap-3 pt-1">
      <span className="font-mono text-[11px] text-ink-3 uppercase tracking-[0.14em]">
        {HERO_COPY.eyebrow}
      </span>
      <h1 className="font-extrabold text-3xl leading-[1.15] tracking-tight md:text-4xl">
        {HERO_COPY.h1Prefix}{' '}
        <span className="font-medium font-serif text-accent italic">
          {HERO_COPY.h1Accent}
        </span>
      </h1>
      <p className="max-w-xl text-ink-3 text-sm">{HERO_COPY.subhead}</p>
    </header>
  )
}

interface StartPanelProps {
  className?: string
  harnesses: readonly string[]
  listening: boolean
  statusText: string
  onPromptCopied: () => void
}

function StartPanel({
  className,
  harnesses,
  listening,
  statusText,
  onPromptCopied,
}: StartPanelProps) {
  return (
    <div
      className={cn(
        'flex flex-col gap-5 rounded-2xl border border-border-2 bg-card p-6',
        className,
      )}
    >
      <AgentChips harnesses={harnesses} />
      <div className="flex flex-col gap-3">
        <div className="flex flex-col gap-1">
          <h2 className="font-semibold text-ink text-lg">
            {PANEL_COPY.heading}
          </h2>
          <p className="text-ink-3 text-sm">{STARTER_PROMPT_LABEL}</p>
        </div>
        <StarterPromptTile
          prompt={STARTER_PROMPT}
          onCopied={onPromptCopied}
          variant="block"
        />
      </div>
      {/* Fixed-height slot: text swaps in place so a copy never reflows the
          panel (and, via items-stretch, the video beside it). */}
      <p className="flex min-h-[2.5rem] items-center gap-2 text-[12.5px] text-ink-3">
        {listening && (
          <span className="size-1.5 shrink-0 animate-pulse rounded-full bg-accent" />
        )}
        <span>{statusText}</span>
      </p>
    </div>
  )
}

function AgentChips({ harnesses }: { harnesses: readonly string[] }) {
  if (harnesses.length === 0) return null
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-1.5 text-ink-2">
        <Check className="size-4 shrink-0 text-accent" />
        <span className="font-mono text-[11.5px] uppercase tracking-[0.08em]">
          {harnesses.length} {CONNECTED_COPY.suffix}
        </span>
      </div>
      <div className="flex flex-wrap gap-1.5">
        {harnesses.map((harness) => (
          <span
            key={harness}
            className="rounded-full border border-border-2 bg-muted px-2.5 py-1 font-mono text-[11px] text-ink-2"
          >
            {harness}
          </span>
        ))}
      </div>
    </div>
  )
}

function OnboardingFooter() {
  return (
    <div className="flex items-center gap-5 pt-1 text-[12.5px]">
      <NavLink
        to={MANAGE_COPY.href}
        onClick={() =>
          track(AnalyticsEvent.OnboardingLinkClicked, { link: 'manage' })
        }
        className="text-ink-2 underline decoration-border-2 underline-offset-2 transition hover:decoration-ink-2"
      >
        {MANAGE_COPY.label}
      </NavLink>
      <a
        href={FOOTER_COPY.docsHref}
        target="_blank"
        rel="noopener noreferrer"
        onClick={() =>
          track(AnalyticsEvent.OnboardingLinkClicked, { link: 'docs' })
        }
        className="text-ink-2 underline decoration-border-2 underline-offset-2 transition hover:decoration-ink-2"
      >
        {FOOTER_COPY.docs}
      </a>
    </div>
  )
}
