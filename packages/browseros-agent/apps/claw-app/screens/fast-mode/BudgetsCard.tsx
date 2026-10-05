/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * How far a single run may go. Both limits end a run by handing back, so the
 * copy says that rather than implying a failure.
 */

import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { jevModeKey, useSetJevBudgets } from '@/modules/api/jev-mode.hooks'
import { budgetsSchema } from './fast-mode.schemas'

interface BudgetsCardProps {
  maxSteps: number
  maxSeconds: number
}

export function BudgetsCard({ maxSteps, maxSeconds }: BudgetsCardProps) {
  // The saved values this draft was taken from, kept alongside it so an update
  // arriving while the card is open can be reconciled during render. Without
  // it the inputs kept stale text, looked dirty against the newer limits, and
  // Save would overwrite them.
  const [draft, setDraft] = useState({
    steps: String(maxSteps),
    seconds: String(maxSeconds),
    fromSteps: maxSteps,
    fromSeconds: maxSeconds,
  })
  const [problem, setProblem] = useState<string | null>(null)
  const save = useSetJevBudgets()
  const queryClient = useQueryClient()

  if (draft.fromSteps !== maxSteps || draft.fromSeconds !== maxSeconds) {
    // An untouched field adopts the new value; an edit in progress is kept and
    // only its baseline moves, so it stays the user's to save or abandon.
    setDraft((current) => ({
      steps:
        current.steps === String(current.fromSteps)
          ? String(maxSteps)
          : current.steps,
      seconds:
        current.seconds === String(current.fromSeconds)
          ? String(maxSeconds)
          : current.seconds,
      fromSteps: maxSteps,
      fromSeconds: maxSeconds,
    }))
  }

  const { steps, seconds } = draft
  const setSteps = (value: string) =>
    setDraft((current) => ({ ...current, steps: value }))
  const setSeconds = (value: string) =>
    setDraft((current) => ({ ...current, seconds: value }))

  const dirty = steps !== String(maxSteps) || seconds !== String(maxSeconds)

  const onSave = async () => {
    const parsed = budgetsSchema.safeParse({
      maxSteps: steps,
      maxSeconds: seconds,
    })
    if (!parsed.success) {
      setProblem(parsed.error.issues[0]?.message ?? 'Check these values.')
      return
    }
    setProblem(null)
    try {
      const next = await save.mutateAsync(parsed.data)
      queryClient.setQueryData(jevModeKey, next)
      toast.success('Limits saved.')
    } catch {
      toast.error('Could not save the limits.')
    }
  }

  return (
    <Card className="flex flex-col gap-4 p-5">
      <div className="flex flex-col gap-1">
        <h2 className="font-medium text-sm">How far one run may go</h2>
        <p className="text-muted-foreground text-sm">
          Reaching either limit stops the run and hands the page back, with
          everything it did so far. Nothing is lost.
        </p>
      </div>
      <div className="flex flex-wrap gap-4">
        <div className="flex flex-col gap-2">
          <Label htmlFor="jev-max-steps">Actions</Label>
          <Input
            id="jev-max-steps"
            className="w-28"
            inputMode="numeric"
            value={steps}
            onChange={(event) => {
              setSteps(event.target.value)
              setProblem(null)
            }}
          />
        </div>
        <div className="flex flex-col gap-2">
          <Label htmlFor="jev-max-seconds">Seconds</Label>
          <Input
            id="jev-max-seconds"
            className="w-28"
            inputMode="numeric"
            value={seconds}
            onChange={(event) => {
              setSeconds(event.target.value)
              setProblem(null)
            }}
          />
        </div>
      </div>
      {problem ? (
        <p className="text-destructive text-sm" role="alert">
          {problem}
        </p>
      ) : null}
      <Button
        className="self-start"
        onClick={onSave}
        disabled={!dirty || save.isPending}
      >
        {save.isPending ? 'Saving' : 'Save limits'}
      </Button>
    </Card>
  )
}
