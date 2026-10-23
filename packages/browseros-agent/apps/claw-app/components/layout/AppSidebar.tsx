/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useHref, useLocation } from 'react-router'
import {
  AnimatedSidebar,
  AnimatedSidebarContent,
  AnimatedSidebarFooter,
  AnimatedSidebarHeader,
  AnimatedSidebarMenu,
  AnimatedSidebarMenuButton,
  AnimatedSidebarMenuItem,
} from '@/components/motion/animated-sidebar'
import { SidebarHelp } from '@/components/sidebar/SidebarHelp'
import { SidebarPrivacy } from '@/components/sidebar/SidebarPrivacy'
import { SidebarThemeToggle } from '@/components/sidebar/SidebarThemeToggle'
import { cn } from '@/lib/utils'
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

function NavLinkButton({ item }: { item: NavItem }) {
  const { pathname } = useLocation()
  const href = useHref(item.to)
  const active = isNavItemActive(item.to, pathname)
  const Icon = item.icon
  return (
    <AnimatedSidebarMenuButton
      href={href}
      isActive={active}
      icon={<Icon className="size-[18px]" strokeWidth={1.75} />}
      className={cn('text-[15px]', active && 'font-semibold text-primary')}
    >
      {item.name}
    </AnimatedSidebarMenuButton>
  )
}

// beUI's menu clones its direct children to attach the hover pill, so each
// item must be an AnimatedSidebarMenuItem rendered right here, not wrapped.
function NavMenu({ items }: { items: NavItem[] }) {
  return (
    <AnimatedSidebarMenu>
      {items.map((item) => (
        <AnimatedSidebarMenuItem key={item.to}>
          <NavLinkButton item={item} />
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
          <NavMenu items={primaryNavItems} />
        </nav>
      </AnimatedSidebarContent>
      <AnimatedSidebarFooter className="gap-1 px-3">
        <nav aria-label="Settings">
          <NavMenu items={secondaryNavItems} />
        </nav>
        <SidebarThemeToggle expanded />
        <SidebarPrivacy expanded />
        <SidebarHelp expanded />
      </AnimatedSidebarFooter>
    </AnimatedSidebar>
  )
}
