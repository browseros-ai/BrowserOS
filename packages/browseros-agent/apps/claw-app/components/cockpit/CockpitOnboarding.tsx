/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The cockpit's first run, until the person's first agent activity (final
 * Paper, Try one on top): neo's one line, one quiet status row (the connected
 * apps as marks, or Get Claude when none were found, and the Claude Cowork
 * step), then "Your turn" with the one prompt to copy, then the demo of that
 * prompt. Cockpit passes the real installed, user-facing harnesses.
 */

import {
  ArrowRight,
  ArrowUpRight,
  CircleCheck,
  LoaderCircle,
} from 'lucide-react'
import { useState } from 'react'
import type { Harness } from '@/components/harness/harness.types'
import { AnimatedBadge } from '@/components/motion/animated-badge'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  FIRST_RUN_COPY,
  GET_CLAUDE_URL,
  TRY_PROMPT,
} from '@/screens/cockpit/cockpit-onboarding.helpers'
import { useAddToClaude } from '@/screens/mcp/claude-desktop.hooks'
import { InstallExtensionDialog } from '@/screens/mcp/InstallExtensionDialog'
import { ConnectedApps } from './ConnectedApps'
import { OnboardingDemo } from './OnboardingDemo'
import { PromptPill } from './PromptPill'

interface CockpitOnboardingProps {
  connectedHarnesses?: readonly Harness[]
}

export function CockpitOnboarding({
  connectedHarnesses = [],
}: CockpitOnboardingProps) {
  const [coworkGuideOpen, setCoworkGuideOpen] = useState(false)
  const found = connectedHarnesses.length > 0
  const line = found ? FIRST_RUN_COPY.ready : FIRST_RUN_COPY.noApps

  return (
    <section className="flex flex-col" aria-label="Getting started">
      <div className="flex items-center gap-3.5">
        <img
          src="/icons/browserclaw.svg"
          alt=""
          aria-hidden
          className="size-10 shrink-0 rounded-[10px]"
        />
        <h1 className="flex flex-wrap items-baseline gap-x-[9px] font-extrabold text-[32px] text-cyanotype-ink leading-10 tracking-[-0.03em]">
          {line.lead && <span>{line.lead}</span>}
          <em className="font-medium font-serif text-[35px] tracking-[-0.01em]">
            {line.accent}
          </em>
          <span>{line.tail}</span>
        </h1>
      </div>

      <div className="flex min-h-10 items-center gap-2.5 pt-2.5 pl-[54px]">
        <div className="flex min-w-0 flex-1 items-center gap-3.5">
          {found ? (
            <ConnectedApps harnesses={connectedHarnesses} />
          ) : (
            <>
              <span className="font-medium text-[15px] text-cyanotype-ink leading-5">
                {FIRST_RUN_COPY.noneFound}
              </span>
              <a
                href={GET_CLAUDE_URL}
                target="_blank"
                rel="noopener noreferrer"
                onClick={() =>
                  track(AnalyticsEvent.OnboardingLinkClicked, {
                    link: 'get-claude',
                  })
                }
                className="inline-flex h-[34px] shrink-0 items-center gap-[7px] rounded-full bg-accent px-[15px] font-semibold text-[14px] text-accent-foreground leading-5 transition-colors hover:bg-accent-2 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-accent/25"
              >
                {FIRST_RUN_COPY.getClaude}
                <ArrowUpRight
                  aria-hidden
                  className="size-[13px]"
                  strokeWidth={2.4}
                />
              </a>
            </>
          )}
        </div>
        <CoworkSetup onOpenGuide={() => setCoworkGuideOpen(true)} />
      </div>

      {/* Paper: 18 px of spacer plus 18 px of padding above the prompt row. */}
      <div className="mt-9 flex flex-wrap items-center gap-2.5">
        <span className="whitespace-nowrap pr-1 font-semibold text-[15px] text-cyanotype-ink leading-5">
          {FIRST_RUN_COPY.tryLabel}
        </span>
        <PromptPill
          prompt={TRY_PROMPT}
          onCopied={() => track(AnalyticsEvent.OnboardingTaskCopied)}
        />
      </div>

      <p className="mt-3 flex flex-wrap items-center gap-2 text-[14px] text-ink-3">
        <AnimatedBadge status="loading" size="sm">
          Listening
        </AnimatedBadge>
        Paste it into {found ? connectedHarnesses[0] : 'Claude'}, press Enter,
        then come back here to watch. This page updates by itself.
      </p>

      <div className="mt-5">
        <OnboardingDemo />
      </div>

      <InstallExtensionDialog
        open={coworkGuideOpen}
        onOpenChange={setCoworkGuideOpen}
      />
    </section>
  )
}

// ---------------------------------------------------------------------------

const LINK_CLASS =
  'group inline-flex shrink-0 cursor-pointer items-center gap-1.5 rounded-md text-[14px] leading-5 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-accent/25'

/** Adds neo to the Claude desktop app from the first-run screen, with the guide as a fallback. */
function CoworkSetup({ onOpenGuide }: { onOpenGuide: () => void }) {
  const claude = useAddToClaude()
  const guide = (label: string) => (
    <button type="button" onClick={onOpenGuide} className={LINK_CLASS}>
      <span className="font-semibold text-accent group-hover:underline group-hover:underline-offset-4">
        {label}
      </span>
    </button>
  )

  if (claude.state === 'connected') {
    return (
      <span className="inline-flex shrink-0 items-center gap-1.5 text-[14px] text-green-ink leading-5">
        <CircleCheck aria-hidden className="size-4" />
        {FIRST_RUN_COPY.coworkConnected}
      </span>
    )
  }
  if (claude.state === 'waiting') {
    return (
      <span className="inline-flex shrink-0 items-center gap-2 text-[14px] leading-5">
        <span
          role="status"
          className="inline-flex items-center gap-1.5 text-ink-2"
        >
          <LoaderCircle aria-hidden className="size-3.5 animate-spin" />
          {FIRST_RUN_COPY.coworkWaiting}
        </span>
        {guide(FIRST_RUN_COPY.coworkGuide)}
      </span>
    )
  }
  if (claude.state === 'stuck') {
    return (
      <span className="inline-flex shrink-0 items-center gap-2 text-[14px] leading-5">
        <span className="text-ink-2">{FIRST_RUN_COPY.coworkStuck}</span>
        {guide(FIRST_RUN_COPY.coworkGuide)}
      </span>
    )
  }
  if (claude.state === 'no-claude') {
    return (
      <a
        href={GET_CLAUDE_URL}
        target="_blank"
        rel="noopener noreferrer"
        className={LINK_CLASS}
      >
        <span className="text-ink-2">{FIRST_RUN_COPY.coworkQuestion}</span>
        <span className="font-semibold text-accent group-hover:underline group-hover:underline-offset-4">
          {FIRST_RUN_COPY.getClaude}
        </span>
        <ArrowUpRight
          aria-hidden
          className="size-3.5 text-accent"
          strokeWidth={2.2}
        />
      </a>
    )
  }
  return (
    <button
      type="button"
      disabled={claude.isOpening}
      onClick={() => {
        track(AnalyticsEvent.OnboardingLinkClicked, { link: 'cowork' })
        claude.add()
      }}
      className={LINK_CLASS}
    >
      <span className="text-ink-2">{FIRST_RUN_COPY.coworkQuestion}</span>
      <span className="font-semibold text-accent group-hover:underline group-hover:underline-offset-4">
        {claude.isOpening
          ? FIRST_RUN_COPY.coworkOpening
          : claude.state === 'update'
            ? FIRST_RUN_COPY.coworkUpdate
            : FIRST_RUN_COPY.coworkAction}
      </span>
      <ArrowRight
        aria-hidden
        className="size-3.5 text-accent transition-transform group-hover:translate-x-0.5"
        strokeWidth={2.2}
      />
    </button>
  )
}
