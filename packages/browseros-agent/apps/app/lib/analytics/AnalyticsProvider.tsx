import { PostHogProvider as Provider } from '@posthog/react'
import type { FC, PropsWithChildren } from 'react'
import { posthog } from './posthog'

/**
 * @public
 */
export const AnalyticsProvider: FC<PropsWithChildren> = ({ children }) => {
  return <Provider client={posthog}>{children}</Provider>
}
