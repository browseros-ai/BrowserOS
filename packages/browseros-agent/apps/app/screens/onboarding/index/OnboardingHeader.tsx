import type { FC } from 'react'
import ProductLogoSvg from '@/assets/product_logo.svg'
import { Button } from '@/components/ui/button'
import { docsUrl, githubOrgUrl } from '@/lib/constants/productUrls'

export interface OnboardingHeaderProps {
  isMounted: boolean
}

export const OnboardingHeader: FC<OnboardingHeaderProps> = ({ isMounted }) => {
  return (
    <header
      className={`border-border/40 border-b transition-all duration-700 ${isMounted ? 'translate-y-0 opacity-100' : '-translate-y-4 opacity-0'}`}
    >
      <div className="mx-auto flex max-w-7xl items-center justify-between px-6 py-4">
        <div className="flex items-center gap-3">
          {/* Floating animation to logo */}
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-accent-orange">
            <img src={ProductLogoSvg} alt="BrowserOS" className="h-6 w-6" />
          </div>
          <span className="font-semibold text-accent-orange text-lg">
            BrowserOS
          </span>
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
