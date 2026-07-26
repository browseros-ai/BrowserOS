import type { FC } from 'react'
import { Outlet } from 'react-router'
import { getProductLogoUrl, PRODUCT_LOGO_ALT } from '@/lib/branding/logo'

const PRODUCT_LOGO_URL = getProductLogoUrl()

export const AuthLayout: FC = () => {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-background p-4">
      <div className="mb-8 flex flex-col items-center">
        <img
          src={PRODUCT_LOGO_URL}
          alt={PRODUCT_LOGO_ALT}
          className="max-h-24 w-auto max-w-xs object-contain sm:max-h-28"
          draggable={false}
        />
      </div>
      <Outlet />
    </div>
  )
}
