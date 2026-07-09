/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Remotion Studio + CLI entry. Registers the FirstRunDemo
 * composition at 1600x900 / 30fps / 20s.
 *
 * Render to MP4 locally with:
 *   bun run video:render      (from apps/claw-app)
 *
 * Render a still for the poster with:
 *   bun run video:poster      (from apps/claw-app)
 *
 * The `component` prop wraps `FirstRunDemo` in the same
 * providers the extension mounts in `entrypoints/newtab/main.tsx`
 * so that any real component reachable from the composition
 * (via `@/components/**`) can call react-query hooks or use
 * react-router primitives without changes:
 *
 *   - `<QueryClientProvider>` with a fresh, non-fetching client.
 *     Later PRs seed specific query keys via
 *     `queryClient.setQueryData()` to render live-looking cockpit
 *     data (e.g. the ready-state activity row).
 *   - `<MemoryRouter>` so `<NavLink>` / `useLocation` render as
 *     inert anchors instead of throwing. The composition never
 *     navigates; it just renders whatever the route defaults to.
 *
 * Providers must live INSIDE the component the Composition
 * renders (not around the Composition itself), because Remotion
 * instantiates that tree fresh on every frame it snapshots.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router'
import { Composition } from 'remotion'
import './index.css'
import { FirstRunDemo } from './FirstRunDemo'
import { FPS, TOTAL_FRAMES } from './timing'

const WIDTH = 1600
const HEIGHT = 900

// One QueryClient for the whole render process. `retry: false` and
// infinite stale/gc times mean any consumer hook will either read
// from a pre-seeded cache entry or return `undefined` synchronously,
// never triggering a network fetch during render.
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: Number.POSITIVE_INFINITY,
      gcTime: Number.POSITIVE_INFINITY,
      retry: false,
    },
  },
})

function FirstRunDemoWithProviders() {
  return (
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>
        <FirstRunDemo />
      </MemoryRouter>
    </QueryClientProvider>
  )
}

export function RemotionRoot() {
  return (
    <Composition
      id="FirstRunDemo"
      component={FirstRunDemoWithProviders}
      durationInFrames={TOTAL_FRAMES}
      fps={FPS}
      width={WIDTH}
      height={HEIGHT}
    />
  )
}
