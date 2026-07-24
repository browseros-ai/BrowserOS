import { useCallback, useEffect, useRef, useState } from 'react'
import { DEFAULT_PLAYBACK_SPEED, PLAYBACK_SPEEDS } from './replay.helpers'

export interface Playback {
  /** Seconds elapsed in the session's global activity window. */
  time: number
  isPlaying: boolean
  speed: number
  setSpeed: (next: number) => void
  /** Starts playback, restarting from zero once activity has completed. */
  play: () => void
  /** Pauses playback without moving the playhead. */
  pause: () => void
  togglePlay: () => void
  /** Jumps the playhead to `seconds` and pauses. Returns the applied value. */
  seek: (seconds: number) => number
}

interface Anchor {
  /** Animation-frame timestamp this run of the clock started from. */
  wallMs: number
  /** Activity time at that timestamp. */
  time: number
}

/**
 * Authoritative clock for session activity time.
 *
 * The app owns this timer rather than rrweb, because the visible tab's
 * recording routinely ends long before the session's last tool call — a
 * player-owned clock simply stops there and strands every later action. Time
 * advances from animation-frame timestamps so pauses, speed changes, and seeks
 * re-anchor exactly instead of accumulating rounding drift.
 */
export function usePlayback(totalSeconds: number): Playback {
  const [time, setTime] = useState(0)
  const [isPlaying, setIsPlaying] = useState(true)
  const [speed, setSpeed] = useState<number>(DEFAULT_PLAYBACK_SPEED)
  const timeRef = useRef(0)
  const anchorRef = useRef<Anchor | null>(null)
  // Distinguishes "stopped because activity ended" from an operator pause, so
  // only the former resumes when a live recording extends the window.
  const completedRef = useRef(false)

  const applyTime = useCallback((next: number) => {
    timeRef.current = next
    setTime(next)
  }, [])

  useEffect(() => {
    if (!isPlaying || totalSeconds <= 0) return
    let active = true
    let frameId = 0
    const tick = (wallMs: number) => {
      if (!active) return
      const anchor = anchorRef.current
      if (anchor === null) {
        // First frame of this run: adopt whatever time the controls left
        // behind, so a speed change never replays or skips elapsed activity.
        anchorRef.current = { wallMs, time: timeRef.current }
      } else {
        const next = anchor.time + ((wallMs - anchor.wallMs) / 1000) * speed
        if (next >= totalSeconds) {
          anchorRef.current = null
          completedRef.current = true
          applyTime(totalSeconds)
          setIsPlaying(false)
          return
        }
        applyTime(next)
      }
      frameId = window.requestAnimationFrame(tick)
    }
    frameId = window.requestAnimationFrame(tick)
    return () => {
      active = false
      window.cancelAnimationFrame(frameId)
      anchorRef.current = null
    }
  }, [applyTime, isPlaying, speed, totalSeconds])

  useEffect(() => {
    if (timeRef.current > totalSeconds) {
      anchorRef.current = null
      applyTime(totalSeconds)
      return
    }
    if (completedRef.current && timeRef.current < totalSeconds) {
      // A live recording grew past where playback stopped; carry on from the
      // accepted time instead of stranding the operator at the old end.
      completedRef.current = false
      anchorRef.current = null
      setIsPlaying(true)
    }
  }, [applyTime, totalSeconds])

  const setPlaybackSpeed = useCallback((next: number) => {
    if (PLAYBACK_SPEEDS.includes(next)) setSpeed(next)
  }, [])

  const play = useCallback(() => {
    anchorRef.current = null
    completedRef.current = false
    if (totalSeconds > 0 && timeRef.current >= totalSeconds) applyTime(0)
    setIsPlaying(true)
  }, [applyTime, totalSeconds])

  const pause = useCallback(() => {
    anchorRef.current = null
    completedRef.current = false
    setIsPlaying(false)
  }, [])

  const togglePlay = useCallback(() => {
    if (isPlaying) pause()
    else play()
  }, [isPlaying, pause, play])

  const seek = useCallback(
    (seconds: number) => {
      const clamped = Math.max(0, Math.min(totalSeconds, seconds))
      anchorRef.current = null
      completedRef.current = false
      applyTime(clamped)
      setIsPlaying(false)
      return clamped
    },
    [applyTime, totalSeconds],
  )

  return {
    time,
    isPlaying,
    speed,
    setSpeed: setPlaybackSpeed,
    play,
    pause,
    togglePlay,
    seek,
  }
}
