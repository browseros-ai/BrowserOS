import { useEffect, useState } from 'react'

/**
 * @public
 */
export const useIsMac = (): boolean | undefined => {
  const [isMac, setIsMac] = useState<boolean | undefined>(undefined)

  useEffect(() => {
    setIsMac(navigator.platform.toUpperCase().indexOf('MAC') >= 0)
  }, [])

  return isMac
}
