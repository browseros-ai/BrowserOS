import { Loader2, Menu } from 'lucide-react'
import type { FC } from 'react'
import { Suspense, useCallback, useEffect, useRef, useState } from 'react'
import { Outlet, useLocation } from 'react-router'
import { AppSidebar } from '@/components/sidebar/AppSidebar'
import { Button } from '@/components/ui/button'
import { Sheet, SheetContent } from '@/components/ui/sheet'
import { useIsMobile } from '@/hooks/use-mobile'
import { RpcClientProvider } from '@/lib/rpc/RpcClientProvider'
import { ShortcutsDialog } from '@/screens/newtab/index/ShortcutsDialog'

const COLLAPSE_DELAY = 150

export const SidebarLayout: FC = () => {
  const location = useLocation()
  const isMobile = useIsMobile()
  const [sidebarOpen, setSidebarOpen] = useState(false)
  const [mobileOpen, setMobileOpen] = useState(false)
  const [shortcutsDialogOpen, setShortcutsDialogOpen] = useState(false)
  const collapseTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const openShortcuts = useCallback(() => {
    setShortcutsDialogOpen(true)
  }, [])

  useEffect(() => {
    setMobileOpen(false)
  }, [])

  useEffect(() => {
    return () => {
      if (collapseTimeoutRef.current) {
        clearTimeout(collapseTimeoutRef.current)
      }
    }
  }, [])

  const handleMouseEnter = useCallback(() => {
    if (collapseTimeoutRef.current) {
      clearTimeout(collapseTimeoutRef.current)
      collapseTimeoutRef.current = null
    }
    setSidebarOpen(true)
  }, [])

  const handleMouseLeave = useCallback(() => {
    collapseTimeoutRef.current = setTimeout(() => {
      setSidebarOpen(false)
    }, COLLAPSE_DELAY)
  }, [])

  const collapseSidebarNow = useCallback(() => {
    if (collapseTimeoutRef.current) {
      clearTimeout(collapseTimeoutRef.current)
      collapseTimeoutRef.current = null
    }
    setSidebarOpen(false)
  }, [])

  if (isMobile) {
    return (
      <RpcClientProvider>
        <div className="flex min-h-screen flex-col bg-background pl-[60px]">
          {/* Always show the left sidebar (collapsed) so it doesn't "disappear" when the
              window is not wide enough for the desktop layout. */}
          <div className="fixed inset-y-0 left-0 z-50">
            <AppSidebar expanded={false} onOpenShortcuts={openShortcuts} />
          </div>
          <header className="flex h-14 shrink-0 items-center gap-2 border-b px-4">
            <Button
              variant="ghost"
              size="icon"
              className="-ml-1 size-7"
              onClick={() => setMobileOpen(true)}
            >
              <Menu className="size-4" />
            </Button>
            <span className="font-semibold">Shimmy</span>
          </header>
          <main className="flex-1 overflow-y-auto">
            <div
              className={[
                'mx-auto max-w-4xl px-4 py-8 transition-all duration-200 ease-in-out sm:px-6 lg:px-8',
                mobileOpen ? 'pr-[420px]' : '',
              ].join(' ')}
            >
              <Outlet />
            </div>
          </main>
          <Sheet open={mobileOpen} onOpenChange={setMobileOpen}>
            <SheetContent side="left" className="w-72 p-0">
              <AppSidebar
                expanded
                onOpenShortcuts={openShortcuts}
                onSidebarAutoCollapse={() => setMobileOpen(false)}
              />
            </SheetContent>
          </Sheet>
          <ShortcutsDialog
            open={shortcutsDialogOpen}
            onOpenChange={setShortcutsDialogOpen}
          />
        </div>
      </RpcClientProvider>
    )
  }

  return (
    <RpcClientProvider>
      {/* pl-14 offsets all content by the collapsed sidebar width (w-14 = 56px) so it never sits under the rail */}
      <div className="relative min-h-screen bg-background pl-14">
        {/* Sidebar - fixed overlay */}
        {/* biome-ignore lint/a11y/noStaticElementInteractions: hover interactions needed */}
        <div
          // Keep the left sidebar above any Home search/glow layers.
          className="fixed inset-y-0 left-0 z-50"
          style={{ width: sidebarOpen ? 256 : 56 }}
          onMouseEnter={handleMouseEnter}
          onMouseLeave={handleMouseLeave}
        >
          <AppSidebar
            expanded={sidebarOpen}
            onOpenShortcuts={openShortcuts}
            onSidebarAutoCollapse={collapseSidebarNow}
          />
        </div>

        {location.pathname === '/home/chat' ? (
          <main className="relative h-dvh overflow-hidden">
            <Suspense
              fallback={
                <div className="flex h-full items-center justify-center">
                  <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
                </div>
              }
            >
              <Outlet />
            </Suspense>
          </main>
        ) : (
          <main
            className="min-h-screen overflow-y-auto transition-all duration-200 ease-in-out"
            style={{
              // Keep content width stable while the sidebar expands.
              // Expanded sidebar overlays content instead of shrinking the center panel.
              paddingLeft: 60,
              paddingRight: 0,
            }}
          >
            <div
              className="mx-auto w-full max-w-4xl px-4 py-8 transition-transform duration-200 ease-in-out sm:px-6 lg:px-8"
              style={{
                // Shift content right without reducing layout width.
                transform: sidebarOpen
                  ? 'translateX(clamp(0px, 12vw, 200px))'
                  : 'translateX(0px)',
              }}
            >
              <Suspense
                fallback={
                  <div className="flex h-40 items-center justify-center">
                    <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
                  </div>
                }
              >
                <Outlet />
              </Suspense>
            </div>
          </main>
        )}
      </div>
      <ShortcutsDialog
        open={shortcutsDialogOpen}
        onOpenChange={setShortcutsDialogOpen}
      />
    </RpcClientProvider>
  )
}
