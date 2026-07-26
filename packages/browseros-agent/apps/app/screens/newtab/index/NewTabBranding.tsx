import { motion } from 'motion/react'
import type { FC } from 'react'
import { getProductLogoUrl, PRODUCT_LOGO_ALT } from '@/lib/branding/logo'

const PRODUCT_LOGO_URL = getProductLogoUrl()

export const NewTabBranding: FC = () => {
  return (
    <div className="space-y-4 text-center">
      <div className="mb-2 flex items-center justify-center gap-3 px-2">
        <motion.div
          layoutId="new-tab-branding"
          transition={{
            type: 'keyframes',
            damping: 20,
            stiffness: 300,
          }}
          className="flex max-h-32 min-h-[7rem] w-full max-w-md items-center justify-center rounded-xl bg-transparent sm:max-h-36"
        >
          <img
            src={PRODUCT_LOGO_URL}
            alt={PRODUCT_LOGO_ALT}
            className="max-h-28 w-auto max-w-full object-contain sm:max-h-32"
            draggable={false}
          />
        </motion.div>
      </div>
    </div>
  )
}
