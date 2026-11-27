/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

/**
 * A sentence with one phrase set in bold blue, which the ultrafast feature
 * cards, the tour and the profile picture share. The phrase never breaks
 * across rows. A <mark>, so it still reads as the point of the sentence.
 */
export function Highlight({ text, phrase }: { text: string; phrase?: string }) {
  const at = phrase ? text.indexOf(phrase) : -1
  if (!phrase || at < 0) return text
  return (
    <>
      {text.slice(0, at)}
      <mark className="whitespace-nowrap bg-transparent font-semibold text-primary">
        {phrase}
      </mark>
      {text.slice(at + phrase.length)}
    </>
  )
}
