import type { FC } from 'react'
import { Button } from '@/components/ui/button'
import { getProductLogoUrl, PRODUCT_LOGO_ALT } from '@/lib/branding/logo'
import { docsUrl, githubOrgUrl } from '@/lib/constants/productUrls'

const PRODUCT_LOGO_URL = getProductLogoUrl()

export interface OnboardingHeaderProps {
  isMounted: boolean
}

export const OnboardingHeader: FC<OnboardingHeaderProps> = ({ isMounted }) => {
  return (
    <header
      className={`border-border/40 border-b transition-all duration-700 ${isMounted ? 'translate-y-0 opacity-100' : '-translate-y-4 opacity-0'}`}
    >
      <div className="mx-auto flex max-w-7xl items-center justify-between px-6 py-4">
        <div className="flex min-w-0 items-center gap-3">
          <img
            src={PRODUCT_LOGO_URL}
            alt={PRODUCT_LOGO_ALT}
            className="h-9 w-auto max-w-[min(100vw-12rem,280px)] object-contain object-left sm:h-10"
            draggable={false}
          />
        </div>
        <nav className="hidden items-center gap-1 md:flex">
          <Button
            asChild
            variant="ghost"
            size="sm"
            className="text-muted-foreground hover:text-foreground"
          >
            <a href={docsUrl} target="_blank" rel="noopener noreferrer">
              Docs
            </a>
          </Button>
          <Button
            asChild
            variant="ghost"
            size="sm"
            className="text-muted-foreground hover:text-foreground"
          >
            <a href={githubOrgUrl} target="_blank" rel="noopener noreferrer">
              GitHub
            </a>
          </Button>
        </nav>
      </div>
    </header>
  )
}
