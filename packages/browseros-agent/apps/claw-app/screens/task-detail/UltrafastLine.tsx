/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { SessionSummary } from '@browseros/claw-api'
import { agentIdentity } from '@/components/harness/harness.types'
import { ultrafastLineText } from './task-detail.helpers'

/** What ultrafast did in this task: a tinted line under the title, ruled in Signal Blue. */
export function UltrafastLine({ session }: { session: SessionSummary }) {
  const agent = agentIdentity(session.harness, session.label || session.slug)
  const text = ultrafastLineText(session.pursue, session.ultrafast, agent.name)
  if (!text) return null

  return (
    <div
      role="note"
      aria-label="What ultrafast did in this task"
      className="relative flex flex-col items-start gap-2.5 rounded-[10px] border border-accent-tint-2 border-l-0 bg-accent-tint py-3 pr-4 pl-[18px] before:absolute before:-inset-y-px before:left-0 before:w-[3px] before:rounded-l-[10px] before:bg-primary md:flex-row md:flex-wrap md:items-center md:justify-between md:gap-x-7 md:py-[15px] md:pr-[22px] md:pl-6 dark:border-primary/25 dark:bg-primary/12"
    >
      <p className="min-w-0 text-pretty text-[14.5px] text-ink-2 leading-normal md:flex-[1_1_420px] md:text-balance md:text-[15px]">
        <span className="mr-[7px] font-bold text-accent-ink italic tracking-[-0.01em]">
          ultrafast
        </span>
        {text.sentence.map((part, index) =>
          typeof part === 'string' ? (
            part
          ) : (
            <span
              // biome-ignore lint/suspicious/noArrayIndexKey: the sentence never reorders
              key={index}
              className="whitespace-nowrap font-medium text-ink tabular-nums"
            >
              {part.figure}
            </span>
          ),
        )}
      </p>
      {text.figures.length > 0 ? (
        <p className="flex flex-wrap items-baseline gap-x-[17px] gap-y-1.5 text-[13px] text-ink-2 md:flex-none md:gap-x-[22px] md:text-[13.5px]">
          {text.figures.map((figure, index) => (
            <span
              key={figure.label}
              className={
                index > 0
                  ? 'relative whitespace-nowrap before:absolute before:inset-y-[3px] before:-left-[9px] before:w-px before:bg-accent-tint-2 md:before:-left-[11px] dark:before:bg-primary/30'
                  : 'whitespace-nowrap'
              }
            >
              <span className="mr-[3px] font-medium text-[13.5px] text-ink tabular-nums md:mr-1 md:text-[14px]">
                {figure.value}
              </span>
              {figure.label}
            </span>
          ))}
        </p>
      ) : null}
    </div>
  )
}
