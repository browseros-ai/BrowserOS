/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Link } from 'react-router'

/** Marks a locked feature and leads to the neo Pro page, never a dead button. */
export function ProTag() {
  return (
    <Link
      to="/pro"
      className="inline-flex h-5 items-center rounded-full bg-accent-tint px-2 font-bold text-[11px] text-accent-ink outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
    >
      Pro
    </Link>
  )
}
