import { Menu } from 'lucide-react'
import type { FC } from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { Outlet, useLocation } from 'react-router'
import { AppSidebar } from '@/components/sidebar/AppSidebar'
import { Button } from '@/components/ui/button'
import { Sheet, SheetContent } from '@/components/ui/sheet'
import { useIsMobile } from '@/hooks/use-mobile'
import { RpcClientProvider } from '@/lib/rpc/RpcClientProvider'
import { cn } from '@/lib/utils'
import { ActiveConversationProvider } from '@/modules/conversations/active-conversation-context'
import { ShortcutsDialog } from '@/screens/newtab/index/ShortcutsDialog'

const COLLAPSE_DELAY = 150

const SidebarLayoutContent: FC = () => {
  const location = useLocation()
  const isChatPage = location.pathname === '/home/chat'
  const isMobile = useIsMobile()
  const [sidebarOpen, setSidebarOpen] = useState(false)
  const [mobileOpen, setMobileOpen] = useState(false)
  const [shortcutsDialogOpen, setShortcutsDialogOpen] = useState(false)
  const sidebarRef = useRef<HTMLDivElement>(null)
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
      // Keyboard focus keeps the rail open even when the pointer leaves it.
      if (!sidebarRef.current?.contains(document.activeElement))
        setSidebarOpen(false)
    }, COLLAPSE_DELAY)
  }, [])

  if (isMobile) {
    return (
      <RpcClientProvider>
        <div
          className={cn(
            'flex flex-col bg-background',
            isChatPage ? 'h-dvh' : 'min-h-screen',
          )}
        >
          <header className="flex h-14 shrink-0 items-center gap-2 border-b px-4">
            <Button
              variant="ghost"
              size="icon"
              className="-ml-1 size-7"
              onClick={() => setMobileOpen(true)}
              aria-label="Open navigation"
            >
              <Menu className="size-4" />
            </Button>
            <span className="font-semibold">BrowserOS</span>
          </header>
          {isChatPage ? (
            <main className="relative min-h-0 flex-1 overflow-hidden">
              <Outlet />
            </main>
          ) : (
            <main className="flex-1 overflow-y-auto">
              <div className="mx-auto max-w-4xl px-4 py-8 sm:px-6 lg:px-8">
                <Outlet />
              </div>
            </main>
          )}
          <Sheet open={mobileOpen} onOpenChange={setMobileOpen}>
            <SheetContent side="left" className="w-72 p-0">
              <AppSidebar
                expanded
                onOpenShortcuts={openShortcuts}
                onNavigate={() => setMobileOpen(false)}
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
          ref={sidebarRef}
          className="fixed inset-y-0 left-0 z-40"
          onMouseEnter={handleMouseEnter}
          onMouseLeave={handleMouseLeave}
          onFocusCapture={handleMouseEnter}
          onBlurCapture={(event) => {
            if (
              !event.currentTarget.contains(event.relatedTarget) &&
              !event.currentTarget.matches(':hover')
            )
              handleMouseLeave()
          }}
        >
          <AppSidebar expanded={sidebarOpen} onOpenShortcuts={openShortcuts} />
        </div>

        {isChatPage ? (
          // The expanded rail adds 200px beyond the 56px already reserved.
          // Keep the transcript readable while browsing history in narrow windows.
          <main
            className={cn(
              'relative h-dvh overflow-hidden transition-[margin] duration-200',
              sidebarOpen && 'ml-[200px]',
            )}
          >
            <Outlet />
          </main>
        ) : (
          <main className="min-h-screen overflow-y-auto">
            <div className="mx-auto max-w-4xl px-4 py-8 sm:px-6 lg:px-8">
              <Outlet />
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

export const SidebarLayout: FC = () => (
  <ActiveConversationProvider>
    <SidebarLayoutContent />
  </ActiveConversationProvider>
)
