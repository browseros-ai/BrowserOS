/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { SessionPursueRun } from '@browseros/claw-api'
import { Card } from '@/components/motion/card'
import { pursueStepLabel } from '@/screens/audit/audit.helpers'

/** The steps neo took on the page for the agent, and where it handed back. */
export function NeoSteps({ pursue }: { pursue: SessionPursueRun }) {
  const handedBack = pursue.outcome === 'handed_back'
  return (
    <section
      aria-labelledby="neo-steps-title"
      className="flex flex-col gap-3.5"
    >
      <h2
        id="neo-steps-title"
        className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]"
      >
        What neo did for your agent
      </h2>
      <Card
        size="compact"
        className="gap-3 border-border-2 bg-card px-4 py-4 shadow-card sm:px-[18px]"
      >
        {pursue.steps.length > 0 ? (
          <ol className="flex flex-col gap-2">
            {pursue.steps.map((step, index) => (
              <li
                // biome-ignore lint/suspicious/noArrayIndexKey: steps never reorder
                key={index}
                className="flex items-baseline gap-2.5 text-[14.5px] text-ink-2"
              >
                <span
                  aria-hidden
                  className="inline-grid size-5 shrink-0 place-items-center rounded-full bg-accent-tint font-bold text-[12px] text-primary"
                >
                  {index + 1}
                </span>
                <span className="text-ink">{pursueStepLabel(step)}</span>
              </li>
            ))}
          </ol>
        ) : null}
        {handedBack ? (
          <p className="text-[13.5px] text-ink-3">
            {pursue.steps.length > 0
              ? 'Then neo handed the page back to your agent.'
              : 'neo handed the page back to your agent before taking a step.'}
          </p>
        ) : (
          <p className="text-[13.5px] text-ink-3">
            neo reached the goal on its own.
          </p>
        )}
      </Card>
    </section>
  )
}
