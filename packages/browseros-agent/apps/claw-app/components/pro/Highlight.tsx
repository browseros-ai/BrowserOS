/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

/**
 * A sentence with one phrase in the yellow highlight that the ultrafast
 * feature cards, the tour and the profile picture share. The phrase never
 * breaks across rows. Its yellow is a one-off, so the colours live here, one
 * pair per theme, rather than in the theme tokens.
 */
export function Highlight({ text, phrase }: { text: string; phrase?: string }) {
  const at = phrase ? text.indexOf(phrase) : -1
  if (!phrase || at < 0) return text
  return (
    <>
      {text.slice(0, at)}
      <mark className="whitespace-nowrap rounded-[4px] bg-[#ffe169] px-[5px] font-semibold text-[#0c2742] dark:bg-[#f5d547] dark:text-[#1b1e1e]">
        {phrase}
      </mark>
      {text.slice(at + phrase.length)}
    </>
  )
}
