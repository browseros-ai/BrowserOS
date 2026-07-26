export const JTBD_POPUP_CONSTANTS = {
  // Show popup after this many messages
  MESSAGE_THRESHOLD: 10,
  // Show to 1 in N users (samplingId % N === 0)
  // Set to 1 to show to everyone
  SAMPLING_DIVISOR: 1,
  // Show "don't show again" checkbox after popup has been shown this many times
  DONT_SHOW_AGAIN_AFTER: 2,
} as const
