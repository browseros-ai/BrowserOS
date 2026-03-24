import * as React from 'react'

// This UI is primarily a desktop layout with a fixed left sidebar.
// If we switch to the mobile-sheet sidebar too early (e.g. when users
// resize the window but it's still wide enough), the left sidebar can
// appear "missing". Keep mobile mode for narrower widths only.
const MOBILE_BREAKPOINT = 640

export function useIsMobile() {
  const [isMobile, setIsMobile] = React.useState<boolean | undefined>(undefined)

  React.useEffect(() => {
    const mql = window.matchMedia(`(max-width: ${MOBILE_BREAKPOINT - 1}px)`)
    const onChange = () => {
      setIsMobile(window.innerWidth < MOBILE_BREAKPOINT)
    }
    mql.addEventListener('change', onChange)
    setIsMobile(window.innerWidth < MOBILE_BREAKPOINT)
    return () => mql.removeEventListener('change', onChange)
  }, [])

  return !!isMobile
}
