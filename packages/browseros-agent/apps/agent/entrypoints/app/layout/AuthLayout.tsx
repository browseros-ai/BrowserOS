import type { FC } from 'react'
import { Outlet } from 'react-router'
import { getProductLogoUrl } from '@/lib/branding/logo'

const PRODUCT_LOGO_URL = getProductLogoUrl()

export const AuthLayout: FC = () => {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-background p-4">
      <div className="mb-8 flex flex-col items-center">
        <img src={PRODUCT_LOGO_URL} alt="BrowserOS" className="size-16" />
      </div>
      <Outlet />
    </div>
  )
}
