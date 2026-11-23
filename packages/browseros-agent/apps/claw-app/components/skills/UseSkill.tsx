/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { ReactNode } from 'react'
import { useCopyText } from '@/components/copy/copy.hooks'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import type { Harness } from '@/components/harness/harness.types'
import { CopyCueContent, SkillPhrase } from '@/components/skills/SkillPhrase'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { skillCommand } from '@/screens/skills/skills.helpers'

/**
 * The skill page's one big action: copy the phrase, then paste it into an agent. The whole phrase
 * box is the button, and after a copy the line under it says what to do next.
 */
export function UseSkill({
  name,
  agents,
  children,
}: {
  name: string
  agents: readonly Harness[]
  /** A quiet note under the paste line, such as the free plan's speed nudge. */
  children?: ReactNode
}) {
  const { copied, copy } = useCopyText()
  const phrase = skillCommand(name)
  return (
    <section className="rounded-[22px] border border-border-2 bg-card-tint p-4 shadow-card sm:p-6">
      <button
        type="button"
        aria-label={`Copy ${phrase}`}
        onClick={async () => {
          if (await copy(phrase)) {
            track(AnalyticsEvent.SkillCopied, { surface: 'page' })
          }
        }}
        className="group/use flex w-full items-center gap-3.5 rounded-2xl border border-accent-tint-2 bg-card py-2.5 pr-2.5 pl-4 text-left shadow-card outline-none transition-colors hover:border-primary/40 focus-visible:ring-2 focus-visible:ring-ring sm:pl-[22px]"
      >
        <SkillPhrase
          name={name}
          className="flex-1 text-[16px] leading-[1.4] sm:text-[20px]"
        />
        <span
          aria-hidden
          className={cn(
            'inline-flex h-[52px] shrink-0 items-center gap-2 rounded-[13px] bg-primary px-[18px] font-semibold text-[15.5px] text-primary-foreground transition-colors group-hover/use:bg-primary/90 sm:px-[22px]',
            '[&_svg]:size-[18px]',
          )}
        >
          <CopyCueContent copied={copied} />
        </span>
      </button>
      <div className="mt-4 flex flex-wrap items-center gap-x-3 gap-y-2.5 text-[14.5px]">
        <p
          aria-live="polite"
          className={copied ? 'font-semibold text-cyanotype-ink' : 'text-ink-2'}
        >
          {copied
            ? 'Copied. Now paste it into your agent and send'
            : 'Then paste it into any of your agents'}
        </p>
        {agents.length ? (
          <ul aria-label="Your agents" className="flex flex-wrap gap-1.5">
            {agents.map((agent) => (
              <li
                key={agent}
                className="inline-flex h-[30px] items-center gap-[7px] rounded-full border border-border-2 bg-card pr-[11px] pl-2 font-semibold text-[13px] text-ink-2"
              >
                <HarnessIcon harness={agent} className="size-[15px]" />
                {agent}
              </li>
            ))}
          </ul>
        ) : null}
      </div>
      {children ? (
        <div className="mt-4 border-border-2 border-t border-dashed pt-3.5">
          {children}
        </div>
      ) : null}
    </section>
  )
}
