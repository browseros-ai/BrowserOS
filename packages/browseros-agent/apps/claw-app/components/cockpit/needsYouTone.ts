// The needs-you attention tone. The mockups call for the --amber family, but
// white text on the --amber token fails WCAG AA in dark mode (the dark token is
// a light #e8a33d). The burnt orange #b85c10 below is the brand amber darkened
// to a fixed value that clears AA with white text in both themes, so the
// attention state reads the same regardless of the viewer's theme.
//
// surface: filled caption plate on the running card.
// action:  the primary action on a pale surface (banner, in-control bar).
export const needsYouTone = {
  surface: 'bg-[#b85c10] text-white',
  action: 'bg-[#b85c10] text-white hover:bg-[#a4520e]',
} as const
