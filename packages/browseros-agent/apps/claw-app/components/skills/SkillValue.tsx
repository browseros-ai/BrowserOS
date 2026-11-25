/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Sparkles, Zap } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { SkillValue as Value } from '@/screens/skills/skills.helpers'

/** What a skill has done for this person: a few figures once it has run, an invite before. */
export function SkillValue({ value }: { value: Value }) {
  if (value.kind === 'invite') {
    return (
      <div className="flex items-center gap-3.5 rounded-[18px] border border-accent-tint-2 border-dashed bg-card px-5 py-4 shadow-card">
        <span
          aria-hidden
          className="grid size-10 shrink-0 place-items-center rounded-xl bg-accent-tint text-primary"
        >
          {value.fast ? (
            <Zap className="size-[18px]" />
          ) : (
            <Sparkles className="size-[18px]" />
          )}
        </span>
        <span className="flex min-w-0 flex-col">
          <span className="font-bold text-[15.5px] text-cyanotype-ink">
            {value.title}
          </span>
          <span className="text-[14px] text-ink-2">{value.body}</span>
        </span>
      </div>
    )
  }
  return (
    <dl
      className="grid overflow-hidden rounded-[18px] border border-border-2 bg-card shadow-card"
      style={{
        gridTemplateColumns: `repeat(${value.tiles.length}, minmax(0, 1fr))`,
      }}
    >
      {value.tiles.map((tile, index) => (
        <div
          key={tile.label}
          className={cn(
            'flex min-w-0 flex-col-reverse gap-1 px-4 py-4 sm:px-5',
            index > 0 && 'border-border-2 border-l',
          )}
        >
          <dt className="text-[13.5px] text-ink-3">{tile.label}</dt>
          <dd
            className={cn(
              'whitespace-nowrap font-extrabold text-[22px] tabular-nums leading-[1.1] tracking-[-0.03em] sm:text-[26px]',
              tile.fast ? 'text-accent-ink' : 'text-cyanotype-ink',
            )}
          >
            {tile.value}
            {tile.unit ? (
              <small className="ml-0.5 font-semibold text-[15px] text-ink-2">
                {tile.unit}
              </small>
            ) : null}
          </dd>
        </div>
      ))}
    </dl>
  )
}
