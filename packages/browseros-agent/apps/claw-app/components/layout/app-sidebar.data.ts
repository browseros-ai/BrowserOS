/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Plan } from '@browseros/claw-api'
import {
  Bookmark,
  History,
  House,
  type LucideIcon,
  Plug,
  Settings,
  Sparkles,
} from 'lucide-react'
import type { AnimatedBadgeStatus } from '@/components/motion/animated-badge'
import { isCardTrial } from '@/modules/plan/trial.helpers'

export interface NavItem {
  name: string
  to: string
  icon: LucideIcon
}

/** Daily destinations, in the order people use them. */
export const primaryNavItems: NavItem[] = [
  { name: 'Home', to: '/', icon: House },
  { name: 'Skills', to: '/skills', icon: Bookmark },
  { name: 'Connected agents', to: '/mcp', icon: Plug },
  { name: 'History', to: '/audit', icon: History },
]

export const secondaryNavItems: NavItem[] = [
  { name: 'Settings', to: '/settings', icon: Settings },
]

/** The ultrafast item goes before Settings, only on builds that sell Pro. */
export function secondaryNavItemsFor(plan: Plan | undefined): NavItem[] {
  if (!plan?.available) return secondaryNavItems
  const name = plan.status === 'none' ? 'Get ultrafast' : 'ultrafast'
  return [{ name, to: '/pro', icon: Sparkles }, ...secondaryNavItems]
}

export interface PlanTag {
  label: string
  status: AnimatedBadgeStatus
}

/**
 * The small tag beside ultrafast in the sidebar. Free has none, unless this
 * computer is offered a free trial (`trialDays`, from `trialOfferDays`); the
 * tag can't be hidden and gives way to the plan's own once there is one.
 */
export function planTagFor(
  plan: Plan | undefined,
  trialDays = 0,
): PlanTag | null {
  if (!plan?.available) return null
  switch (plan.status) {
    case 'none':
      return trialDays > 0
        ? { label: `${trialDays} days free`, status: 'success' }
        : null
    case 'active':
      return isCardTrial(plan)
        ? { label: 'Trial', status: 'info' }
        : { label: 'On', status: 'success' }
    case 'ending':
      return { label: 'Ending', status: 'warning' }
    case 'paymentFailed':
      return { label: 'Check card', status: 'warning' }
    case 'unpaid':
      return { label: 'Paused', status: 'neutral' }
    default:
      return null
  }
}

export function isNavItemActive(to: string, pathname: string): boolean {
  if (to === '/') return pathname === '/'
  return pathname === to || pathname.startsWith(`${to}/`)
}
