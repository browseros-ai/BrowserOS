/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The primary action row on the cockpit first-run block. Single
 * definitive CTA: navigate to MCP setup. The copyable prompt sits
 * below in `StarterPromptTile`, which owns its own affordance;
 * duplicating it here would just add noise next to the primary
 * action.
 */

import { ArrowRight, ChevronsUp } from 'lucide-react'
import { NavLink } from 'react-router'

interface FirstRunPrimaryActionsProps {
  installHref: string
  installLabel: string
  installStatus: 'active' | 'done'
}

export function FirstRunPrimaryActions({
  installHref,
  installLabel,
  installStatus,
}: FirstRunPrimaryActionsProps) {
  return (
    <div className="flex">
      <NavLink
        to={installHref}
        className={
          installStatus === 'active'
            ? 'inline-flex items-center gap-2 rounded-xl bg-accent px-6 py-3 font-semibold text-[14px] text-card shadow-card transition hover:brightness-110'
            : 'inline-flex items-center gap-2 rounded-xl border border-border-2 bg-card px-6 py-3 font-semibold text-[14px] text-ink transition hover:border-border-strong'
        }
      >
        {installStatus === 'active' ? <ChevronsUp className="size-4" /> : null}
        {installLabel}
        <ArrowRight className="size-4" />
      </NavLink>
    </div>
  )
}
