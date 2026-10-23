/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { ReactNode } from 'react'
import { Link, useHref, useLocation } from 'react-router'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import { isUserFacingHarness } from '@/components/harness/harness.types'
import { AnimatedBadge } from '@/components/motion/animated-badge'
import {
  AnimatedSidebar,
  AnimatedSidebarContent,
  AnimatedSidebarFooter,
  AnimatedSidebarHeader,
  AnimatedSidebarMenu,
  AnimatedSidebarMenuButton,
  AnimatedSidebarMenuItem,
} from '@/components/motion/animated-sidebar'
import { CARD_TARGET_CLASS, Card } from '@/components/motion/card'
import { SidebarHelp } from '@/components/sidebar/SidebarHelp'
import { SidebarPrivacy } from '@/components/sidebar/SidebarPrivacy'
import { SidebarThemeToggle } from '@/components/sidebar/SidebarThemeToggle'
import { cn } from '@/lib/utils'
import { useLiveSessions } from '@/modules/api/audit.hooks'
import { useConnections } from '@/modules/api/connections.hooks'
import {
  isNavItemActive,
  type NavItem,
  primaryNavItems,
  secondaryNavItems,
} from './app-sidebar.data'

export function SidebarBrand() {
  return (
    <div className="flex items-center gap-2.5 px-1.5">
      <img
        src="/icons/browserclaw.svg"
        alt=""
        aria-hidden
        className="size-[30px] shrink-0 rounded-lg"
      />
      <span className="truncate font-extrabold text-[16px] text-cyanotype-ink tracking-[-0.02em]">
        BrowserOS neo
      </span>
    </div>
  )
}

function NavLinkButton({ item, badge }: { item: NavItem; badge?: ReactNode }) {
  const { pathname } = useLocation()
  const href = useHref(item.to)
  const active = isNavItemActive(item.to, pathname)
  const Icon = item.icon
  return (
    <AnimatedSidebarMenuButton
      href={href}
      isActive={active}
      badge={badge}
      icon={<Icon className="size-[18px]" strokeWidth={1.75} />}
      className={cn('text-[15px]', active && 'font-semibold text-primary')}
    >
      {item.name}
    </AnimatedSidebarMenuButton>
  )
}

// beUI's menu clones its direct children to attach the hover pill, so each
// item must be an AnimatedSidebarMenuItem rendered right here, not wrapped.
function NavMenu({
  items,
  badges = {},
}: {
  items: NavItem[]
  badges?: Record<string, ReactNode>
}) {
  return (
    <AnimatedSidebarMenu>
      {items.map((item) => (
        <AnimatedSidebarMenuItem key={item.to}>
          <NavLinkButton item={item} badge={badges[item.to]} />
        </AnimatedSidebarMenuItem>
      ))}
    </AnimatedSidebarMenu>
  )
}

/**
 * The one sidebar: brand, the daily destinations with words (not icon-only),
 * and Settings with the remaining utilities pinned to the bottom.
 */
export function AppSidebar() {
  return (
    <AnimatedSidebar
      ariaLabel="Sidebar"
      collapsible="none"
      panelClassName="bg-sidebar"
    >
      <AnimatedSidebarHeader className="pt-5 pb-4">
        <SidebarBrand />
      </AnimatedSidebarHeader>
      <AnimatedSidebarContent className="px-3">
        <nav aria-label="Main">
          <NavMenu
            items={primaryNavItems}
            badges={{ '/': <NeedsYouCount /> }}
          />
        </nav>
      </AnimatedSidebarContent>
      <AnimatedSidebarFooter className="gap-1 px-3">
        <nav aria-label="Settings">
          <NavMenu items={secondaryNavItems} />
        </nav>
        <ConnectedAgentsCard />
        <SidebarThemeToggle expanded />
        <SidebarPrivacy expanded />
        <SidebarHelp expanded />
      </AnimatedSidebarFooter>
    </AnimatedSidebar>
  )
}

// ---------------------------------------------------------------------------

/** How many agents are waiting on a person right now, shown on Home. */
function NeedsYouCount() {
  // Same query key as Home's live view, so this adds no extra request there.
  const live = useLiveSessions({ refetchInterval: 5000 })
  const count =
    live.data?.items.filter((session) => session.live?.helpRequest).length ?? 0
  if (count === 0) return null
  return (
    <AnimatedBadge
      status="warning"
      size="sm"
      showIcon={false}
      aria-label={`${count} ${count === 1 ? 'agent needs' : 'agents need'} you`}
    >
      {count}
    </AnimatedBadge>
  )
}

const VISIBLE_AGENT_MARKS = 3

/** The connected agents at a glance, leading to the Connected agents page. */
function ConnectedAgentsCard() {
  const connections = useConnections()
  if (!connections.data) return null
  const connected = connections.data.items
    .filter((c) => c.installed && isUserFacingHarness(c.harness))
    .map((c) => c.harness)
  const label =
    connected.length === 0
      ? 'No agents connected yet'
      : `${connected.length} ${connected.length === 1 ? 'agent' : 'agents'} connected`
  return (
    <Card variant="muted" size="compact" interactive className="mt-2 px-3">
      <div className="flex items-center gap-2.5">
        {connected.length > 0 ? (
          <span className="flex shrink-0" aria-hidden>
            {connected.slice(0, VISIBLE_AGENT_MARKS).map((harness, index) => (
              <span
                key={harness}
                className={cn(
                  'grid size-6 place-items-center rounded-full border-2 border-muted bg-card shadow-[0_0_0_1px_var(--color-border-2)]',
                  index > 0 && '-ml-1.5',
                )}
              >
                <HarnessIcon harness={harness} className="size-3.5" />
              </span>
            ))}
          </span>
        ) : null}
        <Link
          to="/mcp"
          className={cn(
            CARD_TARGET_CLASS,
            'text-[13px] text-ink-2 leading-tight outline-none focus-visible:underline',
          )}
        >
          {label}
        </Link>
      </div>
    </Card>
  )
}
