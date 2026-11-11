/**
 * The Task menu: every agent connection the viewer has seen, newest first, and how the viewer
 * grouped each one (the agent's own task id, its process, or its client name after an idle gap;
 * `TaskIdentification` in `shared/api.ts`). Choosing one switches the whole screen to it.
 */

import { useEffect, useRef, useState } from 'react'
import type { TaskSummary } from '../../shared/api'
import { identification } from '../format'
import { plural } from './units'

export function TaskMenu({
  tasks,
  task,
  onTask,
}: {
  tasks: TaskSummary[]
  task: TaskSummary | null
  onTask: (taskId: string) => void
}) {
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const button = useRef<HTMLButtonElement>(null)
  const list = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const items = list.current?.querySelectorAll<HTMLButtonElement>('button')
    const current = [...(items ?? [])].find(
      (item) => item.getAttribute('aria-checked') === 'true',
    )
    ;(current ?? items?.[0])?.focus()
    const outside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false)
    }
    document.addEventListener('pointerdown', outside)
    return () => document.removeEventListener('pointerdown', outside)
  }, [open])

  const close = () => {
    setOpen(false)
    button.current?.focus()
  }

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === 'Escape' && open) {
      event.preventDefault()
      close()
      return
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    // From the closed button, an arrow opens the menu; opening focuses the current task.
    if (!open) {
      setOpen(true)
      return
    }
    const items = [
      ...(list.current?.querySelectorAll<HTMLButtonElement>('button') ?? []),
    ]
    const at = items.indexOf(document.activeElement as HTMLButtonElement)
    const next =
      event.key === 'ArrowDown'
        ? (at + 1) % items.length
        : (at - 1 + items.length) % items.length
    items[next]?.focus()
  }

  return (
    <div ref={root} className="relative">
      <button
        ref={button}
        type="button"
        aria-haspopup="true"
        aria-expanded={open}
        disabled={tasks.length === 0}
        onClick={() => setOpen(!open)}
        onKeyDown={onKeyDown}
        className="flex shrink-0 items-center gap-1.5 rounded-md border border-control px-2.5 py-[5px] text-xs leading-4 hover:bg-panel disabled:opacity-60"
      >
        <span className="text-ink-2">Task</span>
        <span className="max-w-[220px] truncate font-semibold text-ink">
          {task?.name ?? 'none yet'}
        </span>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path
            d="M2 3.5l3 3 3-3"
            fill="none"
            stroke="#4A5365"
            strokeWidth="1.4"
          />
        </svg>
      </button>
      {open && (
        <div
          ref={list}
          role="radiogroup"
          aria-label="Tasks"
          onKeyDown={onKeyDown}
          className="absolute top-[calc(100%+6px)] right-0 z-20 max-h-[60vh] w-[380px] overflow-auto rounded-lg border border-line bg-white py-1 shadow-[0_12px_32px_#0D17302E]"
        >
          {tasks.map((candidate) => {
            const chosen = candidate.id === task?.id
            const { totals } = candidate
            return (
              // biome-ignore lint/a11y/useSemanticElements: a list of choices, each a full-width row.
              <button
                key={candidate.id}
                type="button"
                role="radio"
                aria-checked={chosen}
                onClick={() => {
                  onTask(candidate.id)
                  close()
                }}
                className={`flex w-full flex-col gap-0.5 px-3 py-2 text-left focus:outline-none focus-visible:bg-panel ${chosen ? 'bg-neo-tint' : 'hover:bg-panel'}`}
              >
                <span className="flex items-center gap-2 text-xs leading-4">
                  <span
                    className={`font-semibold ${chosen ? 'text-neo' : 'text-ink'}`}
                  >
                    {candidate.name}
                  </span>
                  <span className="text-ink-2">
                    {plural(totals.runs, 'run')}
                    {totals.running > 0 && `, ${totals.running} running`}
                    {totals.failed > 0 && `, ${totals.failed} failed`}
                  </span>
                </span>
                <span className="text-[11px] text-ink-2 leading-[14px]">
                  grouped by {identification(candidate.identifiedBy)}
                </span>
              </button>
            )
          })}
        </div>
      )}
    </div>
  )
}
