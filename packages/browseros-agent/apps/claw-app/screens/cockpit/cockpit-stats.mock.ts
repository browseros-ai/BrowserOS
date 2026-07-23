import type { CockpitStats } from '@/components/cockpit/SavedStatsBand'

export const cockpitStatsMock = {
  hasMeasuredStats: true,
  allTime: {
    browserClawTokenEstimate: 172_000,
    screenshotFirstTokenEstimate: 2_582_000,
    rawTokenSavingsEstimate: 2_410_000,
    humanTimeSavedMs: 34_800_000,
    sessionCount: 143,
    toolCallCount: 4_128,
  },
  last30Days: {
    browserClawTokenEstimate: 73_000,
    screenshotFirstTokenEstimate: 1_097_000,
    rawTokenSavingsEstimate: 1_024_000,
    humanTimeSavedMs: 14_700_000,
    sessionCount: 61,
    toolCallCount: 1_740,
  },
  last7Days: {
    browserClawTokenEstimate: 19_000,
    screenshotFirstTokenEstimate: 287_000,
    rawTokenSavingsEstimate: 268_000,
    humanTimeSavedMs: 3_720_000,
    sessionCount: 17,
    toolCallCount: 452,
  },
} satisfies CockpitStats
