/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Goal-driven browsing: an addon that is off until a key is added.
 *
 * Most of the work here is being honest about which of three states the user
 * is in, and saying what each one means for the agents they have connected,
 * because that is the only externally visible consequence of this screen.
 */

import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Card } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { Switch } from '@/components/ui/switch'
import { useConnections } from '@/modules/api/connections.hooks'
import {
  jevModeKey,
  useJevMode,
  useSetJevPaused,
} from '@/modules/api/jev-mode.hooks'
import { BudgetsCard } from './BudgetsCard'
import { agentVisibility, statusOf } from './fast-mode.helpers'
import { KeyCard } from './KeyCard'

export function FastMode() {
  const mode = useJevMode()
  const connections = useConnections()
  const setPaused = useSetJevPaused()
  const queryClient = useQueryClient()

  if (mode.isPending && !mode.data) {
    return (
      <div className="flex flex-col gap-4 p-6">
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-40 w-full" />
        <Skeleton className="h-40 w-full" />
      </div>
    )
  }

  if (mode.isError) {
    return (
      <div className="p-6">
        <Card className="p-5">
          <h1 className="font-medium text-sm">Goal-driven browsing</h1>
          <p className="mt-1 text-muted-foreground text-sm">
            Could not read the current setting. Everything else is unaffected:
            your agents keep working exactly as they do now.
          </p>
        </Card>
      </div>
    )
  }

  const state = mode.data
  const status = statusOf(state)
  const connected = (connections.data?.items ?? []).filter((c) => c.installed)

  const onToggle = async (next: boolean) => {
    try {
      const updated = await setPaused.mutateAsync({ paused: !next })
      queryClient.setQueryData(jevModeKey, updated)
    } catch {
      toast.error('Could not change that.')
    }
  }

  return (
    <div className="flex flex-col gap-4 p-6">
      <Card className="flex flex-col gap-3 p-5">
        <div className="flex items-start justify-between gap-4">
          <div className="flex flex-col gap-1">
            <div className="flex items-center gap-2">
              <h1 className="font-medium text-sm">Goal-driven browsing</h1>
              <Badge variant={status === 'on' ? 'default' : 'secondary'}>
                {status === 'on' ? 'On' : status === 'paused' ? 'Paused' : 'Off'}
              </Badge>
            </div>
            <p className="max-w-prose text-muted-foreground text-sm">
              Give an agent a whole goal instead of one step at a time. The
              steps are decided here with a small decision model, so your
              agent's own model stops paying for every page in between.
            </p>
            <p className="text-muted-foreground text-sm">
              {agentVisibility(status)}
            </p>
          </div>
          {state?.configured ? (
            <Switch
              checked={status === 'on'}
              onCheckedChange={onToggle}
              disabled={setPaused.isPending}
              aria-label="Goal-driven browsing"
            />
          ) : null}
        </div>
        {status === 'on' && connected.length > 0 ? (
          <p className="text-muted-foreground text-sm">
            Available to {connected.map((c) => c.harness).join(', ')}.
          </p>
        ) : null}
      </Card>

      <KeyCard
        configured={state?.configured ?? false}
        fingerprint={state?.fingerprint ?? ''}
      />

      {state?.configured ? (
        <BudgetsCard
          maxSteps={Number(state.budgets.maxSteps)}
          maxSeconds={Number(state.budgets.maxSeconds)}
        />
      ) : null}
    </div>
  )
}
