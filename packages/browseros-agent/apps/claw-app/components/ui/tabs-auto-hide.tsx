/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Thin wrapper around shadcn `Tabs`. When only one item is given,
 * renders that item's content directly with no `TabsList`. When
 * zero, renders nothing. When two or more, renders the full tabs
 * UI. Purpose: on screens that MIGHT need multi-view partitioning
 * (per-tab audit view, agent detail sub-panels, MCP settings
 * sub-sections) we want the tab bar to appear only when it
 * actually carries information. Callers do not have to branch on
 * `items.length` themselves.
 */

import type * as React from 'react'
import { Tabs, TabsContent, TabsList, TabsTrigger } from './tabs'

export interface AutoHideTabsItem {
  /** Stable id used as the `value` for Tabs + React key. */
  id: string
  /** Tab trigger label. Kept as ReactNode so callers can add badges. */
  label: React.ReactNode
  /** Tab body. Kept as ReactNode so callers can compose freely. */
  content: React.ReactNode
}

export interface AutoHideTabsProps {
  items: AutoHideTabsItem[]
  /** Which tab id is selected on mount. Falls back to the first item. */
  defaultId?: string
  /** Passes through to shadcn TabsList's variant (`default` | `line`). */
  listVariant?: 'default' | 'line'
  className?: string
  listClassName?: string
}

export function AutoHideTabs({
  items,
  defaultId,
  listVariant,
  className,
  listClassName,
}: AutoHideTabsProps) {
  if (items.length === 0) return null
  // Wrap the single-item content in a div so `className` still
  // applies (Fragments silently drop it). Keeps behaviour
  // consistent whether the tab bar is hidden or shown.
  if (items.length === 1) {
    return <div className={className}>{items[0]!.content}</div>
  }
  const initial = defaultId ?? items[0]!.id
  return (
    <Tabs defaultValue={initial} className={className}>
      <TabsList variant={listVariant} className={listClassName}>
        {items.map((it) => (
          <TabsTrigger key={it.id} value={it.id}>
            {it.label}
          </TabsTrigger>
        ))}
      </TabsList>
      {items.map((it) => (
        <TabsContent key={it.id} value={it.id}>
          {it.content}
        </TabsContent>
      ))}
    </Tabs>
  )
}
