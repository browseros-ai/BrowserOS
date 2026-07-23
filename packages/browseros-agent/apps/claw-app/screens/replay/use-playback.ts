import { useCallback, useEffect, useRef, useState } from 'react'
import { DEFAULT_PLAYBACK_SPEED, PLAYBACK_SPEEDS } from './replay.helpers'

export interface Playback {
  /** Seconds elapsed on this BrowserClaw-owned transport. */
  time: number
  /** True while the transport clock and visible rrweb sink should run. */
  isPlaying: boolean
  /** Multiplier applied to wall-clock deltas and the visible rrweb player. */
  speed: number
  setSpeed: (next: number) => void
  /** Toggles play/pause. Restarts from 0 if the transport already finished. */
  togglePlay: () => void
  /** Jumps the playhead to `seconds`, pauses, and returns the clamped value. */
  seek: (seconds: number) => number
  /**
   * Compatibility predicate for viewport orchestration. rrweb time is ignored:
   * the renderer is a visual sink and may never write BrowserClaw's clock.
   */
  syncFromPlayer: (seconds: number) => boolean
}

function clampTime(seconds: number, totalSeconds: number): number {
  return Math.max(0, Math.min(totalSeconds, seconds))
}

/**
 * Owns replay time from one monotonic animation-frame loop. rrweb receives
 * projected seeks/play commands elsewhere, but its per-tab timer cannot be the
 * authority for a session clock that continues across player promotion.
 */
export function usePlayback(totalSeconds: number): Playback {
  const [time, setTime] = useState(0)
  const [isPlaying, setIsPlaying] = useState(true)
  const [speed, setSpeed] = useState<number>(DEFAULT_PLAYBACK_SPEED)
  const timeRef = useRef(0)
  const isPlayingRef = useRef(true)
  const speedRef = useRef<number>(DEFAULT_PLAYBACK_SPEED)
  const totalSecondsRef = useRef(totalSeconds)
  const lastNowMsRef = useRef<number | null>(null)

  const writeTime = useCallback((next: number) => {
    timeRef.current = next
    setTime(next)
  }, [])

  const writePlaying = useCallback((next: boolean) => {
    isPlayingRef.current = next
    setIsPlaying(next)
  }, [])

  /**
   * Accounts for elapsed wall time exactly once before any discontinuity.
   * Reusing this for frames, pause, and speed changes prevents stale closures
   * from charging an interval at both its old and new speed.
   */
  const advanceTo = useCallback(
    (nowMs: number): number => {
      const previousNowMs = lastNowMsRef.current
      lastNowMsRef.current = nowMs
      if (!isPlayingRef.current || previousNowMs === null) {
        return timeRef.current
      }

      const elapsedMs = Math.max(0, nowMs - previousNowMs)
      const next = clampTime(
        timeRef.current + (elapsedMs / 1000) * speedRef.current,
        totalSecondsRef.current,
      )
      if (next !== timeRef.current) writeTime(next)
      if (totalSecondsRef.current > 0 && next >= totalSecondsRef.current) {
        writePlaying(false)
      }
      return next
    },
    [writePlaying, writeTime],
  )

  useEffect(() => {
    totalSecondsRef.current = totalSeconds
    const clamped = clampTime(timeRef.current, totalSeconds)
    if (clamped !== timeRef.current) writeTime(clamped)
    if (totalSeconds > 0 && clamped >= totalSeconds) writePlaying(false)
  }, [totalSeconds, writePlaying, writeTime])

  useEffect(() => {
    if (!isPlaying || totalSeconds <= 0) {
      lastNowMsRef.current = null
      return
    }

    let active = true
    let animationFrameId = 0
    lastNowMsRef.current = performance.now()
    const tick = (nowMs: number): void => {
      if (!active) return
      advanceTo(nowMs)
      if (isPlayingRef.current) {
        animationFrameId = window.requestAnimationFrame(tick)
      }
    }
    animationFrameId = window.requestAnimationFrame(tick)
    return () => {
      active = false
      window.cancelAnimationFrame(animationFrameId)
      lastNowMsRef.current = null
    }
  }, [advanceTo, isPlaying, totalSeconds])

  const setPlaybackSpeed = useCallback(
    (next: number) => {
      if (!PLAYBACK_SPEEDS.includes(next) || next === speedRef.current) return
      if (isPlayingRef.current) advanceTo(performance.now())
      speedRef.current = next
      setSpeed(next)
    },
    [advanceTo],
  )

  const togglePlay = useCallback(() => {
    if (isPlayingRef.current) {
      advanceTo(performance.now())
      writePlaying(false)
      lastNowMsRef.current = null
      return
    }

    if (timeRef.current >= totalSecondsRef.current) writeTime(0)
    lastNowMsRef.current = performance.now()
    writePlaying(totalSecondsRef.current > 0)
  }, [advanceTo, writePlaying, writeTime])

  const seek = useCallback(
    (seconds: number) => {
      const clamped = clampTime(seconds, totalSecondsRef.current)
      writeTime(clamped)
      writePlaying(false)
      lastNowMsRef.current = null
      return clamped
    },
    [writePlaying, writeTime],
  )

  const syncFromPlayer = useCallback((_seconds: number) => {
    return isPlayingRef.current
  }, [])

  return {
    time,
    isPlaying,
    speed,
    setSpeed: setPlaybackSpeed,
    togglePlay,
    seek,
    syncFromPlayer,
  }
}
