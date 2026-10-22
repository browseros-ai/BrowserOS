/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The cockpit's first run, until the person's first agent activity (round 7):
 * neo's one line, one quiet status row (the connected apps as marks, or Get
 * Claude when none were found, and the Claude Cowork step), the demo, then two
 * tasks to try. Cockpit passes the real installed, user-facing harnesses.
 */

import { ArrowRight, ArrowUpRight } from 'lucide-react'
import { useState } from 'react'
import type { Harness } from '@/components/harness/harness.types'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  FIRST_RUN_COPY,
  GET_CLAUDE_URL,
  TRY_PROMPTS,
} from '@/screens/cockpit/cockpit-onboarding.helpers'
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
          <span>{found ? FIRST_RUN_COPY.ready : FIRST_RUN_COPY.noApps}</span>
          <em className="font-medium font-serif text-[35px] tracking-[-0.01em]">
            {FIRST_RUN_COPY.lineAccent}
          </em>
          <span>{FIRST_RUN_COPY.lineTail}</span>
        </h1>
      </div>

      <div className="flex min-h-[44px] items-center gap-2.5 pt-2.5 pl-[54px]">
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
        <button
          type="button"
          onClick={() => {
            track(AnalyticsEvent.OnboardingLinkClicked, { link: 'cowork' })
            setCoworkGuideOpen(true)
          }}
          className="group inline-flex shrink-0 cursor-pointer items-center gap-1.5 rounded-md text-[14px] leading-5 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-accent/25"
        >
          <span className="text-ink-2">{FIRST_RUN_COPY.coworkQuestion}</span>
          <span className="font-semibold text-accent group-hover:underline group-hover:underline-offset-4">
            {FIRST_RUN_COPY.coworkAction}
          </span>
          <ArrowRight
            aria-hidden
            className="size-3.5 text-accent transition-transform group-hover:translate-x-0.5"
            strokeWidth={2.2}
          />
        </button>
      </div>

      <div className="mt-5">
        <OnboardingDemo />
      </div>

      <div className="mt-[18px] flex flex-col gap-2">
        <div className="flex items-start gap-3.5">
          <span className="shrink-0 whitespace-nowrap pt-[11px] font-semibold text-[15px] text-ink leading-5">
            {FIRST_RUN_COPY.tryLabel}
          </span>
          <div className="flex min-w-0 flex-1 flex-wrap gap-2.5">
            {TRY_PROMPTS.map((prompt) => (
              <PromptPill
                key={prompt}
                prompt={prompt}
                onCopied={() => track(AnalyticsEvent.OnboardingTaskCopied)}
              />
            ))}
          </div>
        </div>
        <p className="text-[14px] text-ink-3 leading-5">
          {FIRST_RUN_COPY.tryHint}
        </p>
      </div>

      <InstallExtensionDialog
        open={coworkGuideOpen}
        onOpenChange={setCoworkGuideOpen}
      />
    </section>
  )
}
