/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Discovery for goal-driven browsing, placed where someone is already wiring
 * neo up to an agent rather than behind a menu item nobody opens.
 */

import { Link } from 'react-router'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { useJevMode } from '@/modules/api/jev-mode.hooks'
import { statusOf } from '@/screens/fast-mode/fast-mode.helpers'

export function FastModeCard() {
  const mode = useJevMode()
  const status = statusOf(mode.data)

  return (
    <Card className="flex flex-wrap items-center justify-between gap-4 p-5">
      <div className="flex flex-col gap-1">
        <div className="flex items-center gap-2">
          <h2 className="font-medium text-sm">Goal-driven browsing</h2>
          {status === 'on' ? (
            <Badge>On</Badge>
          ) : status === 'paused' ? (
            <Badge variant="secondary">Paused</Badge>
          ) : null}
        </div>
        <p className="max-w-prose text-muted-foreground text-sm">
          {status === 'off'
            ? 'Optional. Add a decision model key and your agents can hand over a whole goal instead of stepping through every page.'
            : 'Your agents can hand over a whole goal instead of stepping through every page.'}
        </p>
      </div>
      <Button variant={status === 'off' ? 'default' : 'outline'} render={
        <Link to="/fast-mode">
          {status === 'off' ? 'Set it up' : 'Manage'}
        </Link>
      } />
    </Card>
  )
}
