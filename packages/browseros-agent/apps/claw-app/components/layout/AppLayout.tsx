/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Menu } from 'lucide-react'
import {
  AnimatedSidebarInset,
  AnimatedSidebarProvider,
  AnimatedSidebarTrigger,
} from '@/components/motion/animated-sidebar'
import { AppSidebar, SidebarBrand } from './AppSidebar'
import { SIDEBAR_WIDTH } from './layout.constants'
import { PageTransition } from './PageTransition'

/**
 * The only application shell. Every route renders inside it: the persistent
 * sidebar (a sheet on small screens), one scrolling document, and the routed
 * page with its enter transition.
 */
export function AppLayout() {
  return (
    // `open` is pinned so the desktop sidebar always shows its labels; the
    // beUI collapse shortcut has nothing to toggle.
    <AnimatedSidebarProvider
      open
      style={{ '--sidebar-width': `${SIDEBAR_WIDTH}px` }}
    >
      <AppSidebar />
      <AnimatedSidebarInset className="bg-transparent">
        <div className="sticky top-0 z-30 flex h-14 items-center gap-2 border-border border-b bg-sidebar px-2 md:hidden">
          <AnimatedSidebarTrigger className="text-ink-2 hover:bg-accent-tint">
            <Menu aria-hidden className="size-5" />
          </AnimatedSidebarTrigger>
          <SidebarBrand />
        </div>
        <PageTransition />
      </AnimatedSidebarInset>
    </AnimatedSidebarProvider>
  )
}
