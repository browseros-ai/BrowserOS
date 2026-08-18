export type ActivityCardCaptionTone = 'light' | 'blue'

export const activityCardCaptionTones = {
  light: {
    surface: 'bg-card text-cyanotype-ink',
    subdued: 'text-cyanotype-soft',
  },
  blue: {
    surface: 'bg-cyanotype-blue text-white',
    subdued: 'text-white/70',
  },
} as const satisfies Record<
  ActivityCardCaptionTone,
  { surface: string; subdued: string }
>
