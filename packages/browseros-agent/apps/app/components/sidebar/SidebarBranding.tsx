import type { FC } from 'react'
import ProductLogo from '@/assets/product_logo.svg'
import { ThemeToggle } from '@/components/elements/theme-toggle'
import { cn } from '@/lib/utils'
import { useWorkspace } from '@/modules/workspace/workspace.hooks'

export interface SidebarBrandingProps {
  expanded?: boolean
}

/**
 * The sidebar header: what you are working in, and the theme toggle.
 *
 * It used to double as the account menu, so the whole header was a dropdown
 * trigger. Every item in that menu was sign in, sign out or profile, so with
 * the account gone the menu had nothing left to open.
 */
export const SidebarBranding: FC<SidebarBrandingProps> = ({
  expanded = true,
}) => {
  const { selectedFolder } = useWorkspace()

  return (
    <div className="flex h-14 items-center justify-between border-b px-2">
      <div className="flex items-center gap-2 p-1.5">
        <img src={ProductLogo} alt="BrowserOS" className="size-8 shrink-0" />
        <span
          className={cn(
            'min-w-0 truncate font-semibold leading-none transition-opacity duration-200',
            expanded ? 'opacity-100' : 'hidden',
          )}
        >
          {selectedFolder?.name || 'BrowserOS'}
        </span>
      </div>
      <div
        className={cn(
          'shrink-0 transition-opacity duration-200',
          expanded ? 'opacity-100' : 'hidden',
        )}
      >
        <ThemeToggle className="h-8 w-8" iconClassName="h-4 w-4" />
      </div>
    </div>
  )
}
