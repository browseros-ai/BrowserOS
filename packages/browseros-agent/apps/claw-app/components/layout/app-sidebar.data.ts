/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  Bookmark,
  History,
  House,
  type LucideIcon,
  Plug,
  Settings,
} from 'lucide-react'

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

export function isNavItemActive(to: string, pathname: string): boolean {
  if (to === '/') return pathname === '/'
  return pathname === to || pathname.startsWith(`${to}/`)
}
