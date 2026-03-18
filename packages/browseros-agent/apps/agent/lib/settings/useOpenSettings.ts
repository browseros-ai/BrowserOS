import { useCallback } from 'react'
import { useLocation, useNavigate } from 'react-router'

/**
 * Hook to open the settings dialog from anywhere in the app.
 * Uses React Router's background location pattern so the dialog
 * overlays the current page without unmounting it.
 */
export function useOpenSettings() {
  const location = useLocation()
  const navigate = useNavigate()

  return useCallback(
    (tab = 'ai') => {
      navigate(`/settings/${tab}`, {
        state: { backgroundLocation: location },
      })
    },
    [location, navigate],
  )
}
