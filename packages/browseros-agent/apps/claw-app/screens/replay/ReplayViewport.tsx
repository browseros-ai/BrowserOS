/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Bounded rrweb stage for session replay. Two physical slots are permanent:
 * one visible active player and at most one hidden, paused standby. Role swaps
 * never create a third player or let a hidden rrweb timer run.
 */

import { Lock } from 'lucide-react'
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { cn } from '@/lib/utils'
import type { ReplayEvent, ReplayFrame } from '@/modules/api/replay.hooks'
import { ReplayPlayer, type ReplayPlayerHandle } from './ReplayPlayer'
import { KIND_STYLE, VERB_META } from './replay.helpers'

export type { ReplayPlayerHandle } from './ReplayPlayer'

export interface ReplayViewportTrack {
  tabId: number
  events: readonly ReplayEvent[]
}

interface ReplayViewportProps {
  site: string
  /** The active tab frame whose caption and URL are currently displayed. */
  frame: ReplayFrame | undefined
  activeTrack: ReplayViewportTrack
  standbyTrack: ReplayViewportTrack | null
  /** Current app-clock projection into the active rrweb stream. */
  activeTimeMs: number
  /** Projection captured when the pending standby was prepared. */
  standbyTimeMs: number
  isPlaying: boolean
  speed: number
  /**
   * Changes only for discontinuities such as seek/resume. Ordinary clock ticks
   * update `activeTimeMs` without forcing rrweb to seek every animation frame.
   */
  syncKey: number
  mode: 'follow' | 'inspect'
}

type SlotId = 'a' | 'b'

interface SlotAssignment {
  track: ReplayViewportTrack | null
  generation: number
}

interface StageState {
  activeSlot: SlotId
  /** A ready target is promoted only after the old active has been paused. */
  promoteSlot: SlotId | null
  slots: Record<SlotId, SlotAssignment>
}

interface HandleEntry {
  generation: number
  handle: ReplayPlayerHandle | null
}

const DRIFT_CHECK_INTERVAL_MS = 1_000
const DRIFT_THRESHOLD_MS = 250

/**
 * Keeps one browser chrome while independent rrweb players swap underneath.
 * BrowserClaw's projected time is authoritative; rrweb is positioned on
 * discontinuities and corrected only for material, throttled drift.
 */
export function ReplayViewport({
  site,
  frame,
  activeTrack,
  standbyTrack,
  activeTimeMs,
  standbyTimeMs,
  isPlaying,
  speed,
  syncKey,
  mode,
}: ReplayViewportProps) {
  const [stage, setStage] = useState<StageState>(() =>
    initialStage(activeTrack, standbyTrack),
  )
  const stageRef = useRef(stage)
  const handlesRef = useRef<Record<SlotId, HandleEntry | null>>({
    a: null,
    b: null,
  })
  const activeTimeMsRef = useRef(activeTimeMs)
  const standbyTimeMsRef = useRef(standbyTimeMs)
  const isPlayingRef = useRef(isPlaying)
  const speedRef = useRef(speed)
  const modeRef = useRef(mode)
  const lastDriftCheckMsRef = useRef(performance.now())
  const appliedSyncKeyRef = useRef<number | null>(null)
  stageRef.current = stage
  activeTimeMsRef.current = activeTimeMs
  standbyTimeMsRef.current = standbyTimeMs
  isPlayingRef.current = isPlaying
  speedRef.current = speed
  modeRef.current = mode

  const writeStage = useCallback(
    (update: (current: StageState) => StageState) => {
      setStage((current) => {
        const next = update(current)
        stageRef.current = next
        return next
      })
    },
    [],
  )

  const promoteReadySlot = useCallback(
    (slot: SlotId) => {
      const current = stageRef.current
      if (current.promoteSlot !== slot) return
      const nextHandle = handleForSlot(current, handlesRef.current, slot)
      if (!nextHandle) return

      const previousHandle = handleForSlot(
        current,
        handlesRef.current,
        current.activeSlot,
      )
      // Promotion order is deliberate: stop the only running rrweb timer,
      // refresh the hidden player's projection, then expose it. Playback is
      // started by the post-swap layout effect, never while the slot is hidden.
      previousHandle?.pause()
      nextHandle.pause()
      nextHandle.seek(activeTimeMsRef.current)
      nextHandle.setSpeed(speedRef.current)
      writeStage((latest) => {
        if (latest.promoteSlot !== slot) return latest
        const oldActiveSlot = latest.activeSlot
        const next: StageState = {
          ...latest,
          activeSlot: slot,
          promoteSlot: null,
        }
        if (modeRef.current === 'inspect') {
          next.slots = assignTrack(next.slots, oldActiveSlot, null)
        }
        return next
      })
    },
    [writeStage],
  )

  const onSlotReady = useCallback(
    (slot: SlotId, generation: number, handle: ReplayPlayerHandle | null) => {
      const current = stageRef.current
      if (current.slots[slot].generation !== generation) return
      handlesRef.current[slot] = { generation, handle }
      if (!handle) return

      handle.setSpeed(speedRef.current)
      if (slot === current.activeSlot && current.promoteSlot !== slot) {
        handle.seek(activeTimeMsRef.current)
        if (isPlayingRef.current) handle.play(activeTimeMsRef.current)
        else handle.pause()
      } else {
        // Standby readiness is never permission to run. It is prepared at the
        // best projection known now and re-seeked immediately before promotion.
        handle.pause()
        handle.seek(
          current.promoteSlot === slot
            ? activeTimeMsRef.current
            : standbyTimeMsRef.current,
        )
      }
      promoteReadySlot(slot)
    },
    [promoteReadySlot],
  )

  useLayoutEffect(() => {
    writeStage((current) =>
      reconcileStage(current, activeTrack, standbyTrack, mode),
    )
  }, [activeTrack, mode, standbyTrack, writeStage])

  useLayoutEffect(() => {
    if (stage.promoteSlot) promoteReadySlot(stage.promoteSlot)
  }, [promoteReadySlot, stage.promoteSlot])

  const activeSlot = stage.activeSlot
  const inactiveSlot = otherSlot(activeSlot)
  const activeGeneration = stage.slots[activeSlot].generation
  const inactiveGeneration = stage.slots[inactiveSlot].generation
  useLayoutEffect(() => {
    const activeEntry = handlesRef.current[activeSlot]
    const inactiveEntry = handlesRef.current[inactiveSlot]
    const activeHandle =
      activeEntry?.generation === activeGeneration ? activeEntry.handle : null
    const inactiveHandle =
      inactiveEntry?.generation === inactiveGeneration
        ? inactiveEntry.handle
        : null
    inactiveHandle?.pause()
    if (!activeHandle) return
    activeHandle.setSpeed(speed)
    if (isPlaying) activeHandle.play(activeTimeMsRef.current)
    else activeHandle.pause()
    lastDriftCheckMsRef.current = performance.now()
  }, [
    activeGeneration,
    activeSlot,
    inactiveGeneration,
    inactiveSlot,
    isPlaying,
    speed,
  ])

  useLayoutEffect(() => {
    if (appliedSyncKeyRef.current === syncKey) return
    appliedSyncKeyRef.current = syncKey
    const current = stageRef.current
    if (!sameTrack(current.slots[current.activeSlot].track, activeTrack)) {
      const oldActiveHandle = handleForSlot(
        current,
        handlesRef.current,
        current.activeSlot,
      )
      const desiredSlot = slotForTrack(current.slots, activeTrack)
      const targetHandle = desiredSlot
        ? handleForSlot(current, handlesRef.current, desiredSlot)
        : null
      oldActiveHandle?.pause()
      targetHandle?.pause()
      targetHandle?.seek(activeTimeMsRef.current)
      lastDriftCheckMsRef.current = performance.now()
      return
    }
    if (current.promoteSlot) {
      const oldActiveHandle = handleForSlot(
        current,
        handlesRef.current,
        current.activeSlot,
      )
      const targetHandle = handleForSlot(
        current,
        handlesRef.current,
        current.promoteSlot,
      )
      oldActiveHandle?.pause()
      targetHandle?.pause()
      targetHandle?.seek(activeTimeMsRef.current)
      promoteReadySlot(current.promoteSlot)
      lastDriftCheckMsRef.current = performance.now()
      return
    }
    const activeHandle = handleForSlot(
      current,
      handlesRef.current,
      current.activeSlot,
    )
    const inactiveHandle = handleForSlot(
      current,
      handlesRef.current,
      otherSlot(current.activeSlot),
    )
    activeHandle?.seek(activeTimeMsRef.current)
    inactiveHandle?.pause()
    inactiveHandle?.seek(standbyTimeMsRef.current)
    if (activeHandle && isPlayingRef.current) {
      activeHandle.play(activeTimeMsRef.current)
    }
    lastDriftCheckMsRef.current = performance.now()
  }, [activeTrack, promoteReadySlot, syncKey])

  useEffect(() => {
    if (!isPlaying) return
    const nowMs = performance.now()
    if (nowMs - lastDriftCheckMsRef.current < DRIFT_CHECK_INTERVAL_MS) return
    lastDriftCheckMsRef.current = nowMs
    const current = stageRef.current
    const handle = handleForSlot(
      current,
      handlesRef.current,
      current.activeSlot,
    )
    if (
      !handle ||
      Math.abs(handle.getCurrentTime() - activeTimeMs) <= DRIFT_THRESHOLD_MS
    ) {
      return
    }
    handle.seek(activeTimeMs)
    handle.play(activeTimeMs)
  }, [activeTimeMs, isPlaying])

  useEffect(
    () => () => {
      for (const entry of Object.values(handlesRef.current)) {
        entry?.handle?.pause()
      }
      handlesRef.current = { a: null, b: null }
    },
    [],
  )

  const addressBar = frame?.url ?? site
  return (
    <div className="relative flex flex-1 flex-col overflow-hidden rounded-2xl border border-border-2 bg-card shadow-sm">
      <Chrome url={addressBar} />
      <div className="relative flex flex-1 items-stretch justify-center overflow-hidden bg-bg-sunken">
        {(['a', 'b'] as const).map((slot) => {
          const assignment = stage.slots[slot]
          const active = slot === stage.activeSlot
          return (
            <div
              key={slot}
              aria-hidden={!active}
              data-replay-slot={slot}
              data-replay-slot-active={active}
              data-replay-tab-id={assignment.track?.tabId}
              className={cn(
                'absolute inset-0',
                active
                  ? 'visible z-0 opacity-100'
                  : 'pointer-events-none invisible -z-10 opacity-0',
              )}
            >
              <PlayerSlot
                slot={slot}
                assignment={assignment}
                onReady={onSlotReady}
              />
            </div>
          )
        })}
        {frame && <Caption frame={frame} />}
      </div>
    </div>
  )
}

function PlayerSlot({
  slot,
  assignment,
  onReady,
}: {
  slot: SlotId
  assignment: SlotAssignment
  onReady: (
    slot: SlotId,
    generation: number,
    handle: ReplayPlayerHandle | null,
  ) => void
}) {
  const { generation, track } = assignment
  const reportReady = useCallback(
    (handle: ReplayPlayerHandle | null) => onReady(slot, generation, handle),
    [generation, onReady, slot],
  )
  return <ReplayPlayer events={track?.events ?? []} onReady={reportReady} />
}

function initialStage(
  activeTrack: ReplayViewportTrack,
  standbyTrack: ReplayViewportTrack | null,
): StageState {
  return {
    activeSlot: 'a',
    promoteSlot: null,
    slots: {
      a: { track: activeTrack, generation: 1 },
      b: { track: standbyTrack, generation: standbyTrack ? 1 : 0 },
    },
  }
}

/**
 * Reconciliation changes only slot assignments and desired roles. Imperative
 * promotion waits for readiness, which keeps the prior active slot visible if
 * a new standby is still constructing or reports readiness out of order.
 */
function reconcileStage(
  current: StageState,
  activeTrack: ReplayViewportTrack,
  standbyTrack: ReplayViewportTrack | null,
  mode: ReplayViewportProps['mode'],
): StageState {
  let next = current
  const matchingActiveSlot = slotForTrack(current.slots, activeTrack)
  if (matchingActiveSlot === current.activeSlot) {
    if (current.promoteSlot !== null) {
      next = { ...next, promoteSlot: null }
    }
  } else if (matchingActiveSlot) {
    next = { ...next, promoteSlot: matchingActiveSlot }
  } else if (!current.slots[current.activeSlot].track) {
    next = {
      ...next,
      slots: assignTrack(next.slots, current.activeSlot, activeTrack),
    }
  } else {
    const targetSlot = otherSlot(current.activeSlot)
    next = {
      ...next,
      promoteSlot: targetSlot,
      slots: assignTrack(next.slots, targetSlot, activeTrack),
    }
  }

  const inactiveSlot = otherSlot(next.activeSlot)
  if (mode === 'inspect') {
    if (next.promoteSlot === null) {
      next = {
        ...next,
        slots: assignTrack(next.slots, inactiveSlot, null),
      }
    }
    return sameStage(current, next) ? current : next
  }

  if (
    standbyTrack &&
    !sameTrack(standbyTrack, activeTrack) &&
    next.promoteSlot === null
  ) {
    next = {
      ...next,
      slots: assignTrack(next.slots, inactiveSlot, standbyTrack),
    }
  }
  return sameStage(current, next) ? current : next
}

function assignTrack(
  slots: StageState['slots'],
  slot: SlotId,
  track: ReplayViewportTrack | null,
): StageState['slots'] {
  if (sameTrack(slots[slot].track, track)) return slots
  return {
    ...slots,
    [slot]: {
      track,
      generation: slots[slot].generation + 1,
    },
  }
}

function slotForTrack(
  slots: StageState['slots'],
  track: ReplayViewportTrack,
): SlotId | null {
  if (sameTrack(slots.a.track, track)) return 'a'
  if (sameTrack(slots.b.track, track)) return 'b'
  return null
}

function sameTrack(
  left: ReplayViewportTrack | null,
  right: ReplayViewportTrack | null,
): boolean {
  return (
    left === right ||
    (left !== null &&
      right !== null &&
      left.tabId === right.tabId &&
      left.events === right.events)
  )
}

function sameStage(left: StageState, right: StageState): boolean {
  return (
    left.activeSlot === right.activeSlot &&
    left.promoteSlot === right.promoteSlot &&
    left.slots === right.slots
  )
}

function otherSlot(slot: SlotId): SlotId {
  return slot === 'a' ? 'b' : 'a'
}

function handleForSlot(
  stage: StageState,
  handles: Record<SlotId, HandleEntry | null>,
  slot: SlotId,
): ReplayPlayerHandle | null {
  const entry = handles[slot]
  return entry?.generation === stage.slots[slot].generation
    ? entry.handle
    : null
}

function Chrome({ url }: { url: string }) {
  return (
    <div className="flex h-9 shrink-0 items-center gap-2 border-border border-b bg-bg-sunken px-3">
      <span className="flex gap-1.5">
        <span className="size-2.5 rounded-full bg-[#FF5F57]" />
        <span className="size-2.5 rounded-full bg-[#FEBC2E]" />
        <span className="size-2.5 rounded-full bg-[#28C840]" />
      </span>
      <div className="ml-3 flex h-6 flex-1 items-center gap-2 rounded-md border border-border-2 bg-card px-3 font-mono text-ink-2 text-xs">
        <Lock className="size-3 text-ink-3" />
        <span className="truncate">{url}</span>
      </div>
    </div>
  )
}

function Caption({ frame }: { frame: ReplayFrame }) {
  const verb = VERB_META[frame.verb]
  const kind = KIND_STYLE[frame.kind]
  return (
    <div className="absolute bottom-5 left-1/2 z-10 flex max-w-[82%] -translate-x-1/2 items-center gap-2.5 rounded-full bg-ink-deep/90 px-4 py-2 shadow-xl backdrop-blur">
      <span
        className={cn(
          'flex size-5 items-center justify-center rounded-md text-white',
          kind.dotClass,
        )}
      >
        <verb.Icon className="size-3" />
      </span>
      <span className="truncate font-semibold text-white/90 text-xs">
        {frame.caption}
      </span>
    </div>
  )
}
