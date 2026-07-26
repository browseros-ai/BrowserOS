import { ChevronDown, LogIn, LogOut, User } from 'lucide-react'
import type { FC } from 'react'
import { useNavigate } from 'react-router'
import { ThemeToggle } from '@/components/elements/theme-toggle'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { getProductLogoUrl, PRODUCT_LOGO_ALT } from '@/lib/branding/logo'
import { cn } from '@/lib/utils'
import { useGraphqlQuery } from '@/modules/graphql/graphql-query.hooks'
import { useWorkspace } from '@/modules/workspace/workspace.hooks'
import { GetProfileByUserIdDocument } from '@/screens/profile/graphql/profileDocument'

const PRODUCT_LOGO_URL = getProductLogoUrl()

export interface SidebarBrandingProps {
  expanded?: boolean
}

export const SidebarBranding: FC<SidebarBrandingProps> = ({
  expanded = true,
}) => {
  const { selectedFolder } = useWorkspace()
  const { sessionInfo } = useSessionInfo()
  const navigate = useNavigate()

  const user = sessionInfo?.user
  const isLoggedIn = !!user

  const { data: profileData } = useGraphqlQuery(
    GetProfileByUserIdDocument,
    { userId: user?.id ?? '' },
    { enabled: !!user?.id },
  )

  const profile = profileData?.profileByUserId
  const profileName =
    profile?.firstName || profile?.lastName
      ? [profile.firstName, profile.lastName].filter(Boolean).join(' ')
      : null
  const displayName = profileName || user?.name || 'User'
  const displayImage = profile?.avatarUrl || user?.image

  const getInitials = (name?: string | null) => {
    if (!name) return '?'
    return name
      .split(' ')
      .map((n) => n[0])
      .join('')
      .toUpperCase()
      .slice(0, 2)
  }

  const headerIcon = isLoggedIn ? (
    displayImage ? (
      <img
        src={displayImage}
        alt={displayName}
        className="size-10 shrink-0 rounded-full object-cover"
      />
    ) : (
      <div className="flex size-10 shrink-0 items-center justify-center rounded-full bg-primary font-medium text-primary-foreground text-xs">
        {getInitials(displayName)}
      </div>
    )
  ) : (
    <img
      src={PRODUCT_LOGO_URL}
      alt={PRODUCT_LOGO_ALT}
      className={cn(
        'h-10 shrink-0 object-contain',
        expanded ? 'w-auto max-w-40 object-left' : 'w-10 object-center',
      )}
      draggable={false}
    />
  )

  return (
    <div
      className={cn(
        'flex h-14 items-center border-b px-2',
        expanded ? 'justify-between' : 'justify-center',
      )}
    >
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className={cn(
              'flex cursor-pointer items-center transition-colors hover:bg-sidebar-accent focus-visible:outline-none',
              expanded
                ? 'w-full gap-2 rounded-lg p-1.5 pr-3 text-left'
                : 'size-10 justify-center rounded-lg',
            )}
          >
            {headerIcon}
            <div
              className={cn(
                'flex min-w-0 flex-col gap-0.5 leading-none transition-opacity duration-200',
                expanded ? 'opacity-100' : 'hidden',
              )}
            >
              <div className="flex items-center gap-1">
                <span className="truncate font-semibold">
                  {isLoggedIn ? displayName : selectedFolder?.name || 'Shimmy'}
                </span>
                <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" />
              </div>
              <span
                className={cn(
                  'truncate text-xs',
                  isLoggedIn
                    ? 'text-muted-foreground'
                    : 'font-medium text-primary',
                )}
              >
                {isLoggedIn ? 'Personal' : 'Sign in'}
              </span>
            </div>
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          side={expanded ? 'bottom' : 'right'}
          align="start"
          className="w-56"
        >
          {isLoggedIn ? (
            <>
              <DropdownMenuLabel className="font-normal">
                <div className="flex flex-col space-y-1">
                  <p className="font-medium text-sm leading-none">
                    {displayName}
                  </p>
                  <p className="text-muted-foreground text-xs leading-none">
                    Personal
                  </p>
                </div>
              </DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuItem onClick={() => navigate('/profile')}>
                <User className="mr-2 size-4" />
                Update Profile
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                onClick={() => navigate('/logout')}
                variant="destructive"
              >
                <LogOut className="mr-2 size-4" />
                Sign out
              </DropdownMenuItem>
            </>
          ) : (
            <DropdownMenuItem onClick={() => navigate('/login')}>
              <LogIn className="mr-2 size-4" />
              Sign in
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
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
