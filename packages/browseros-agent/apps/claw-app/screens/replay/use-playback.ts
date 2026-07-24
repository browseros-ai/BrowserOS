import { useCallback, useEffect, useState } from 'react'
import { DEFAULT_PLAYBACK_SPEED, PLAYBACK_SPEEDS } from './replay.helpers'

export interface Playback {
  /** Seconds elapsed in the session. */
  time: number
  /** True while rrweb's internal timer should be running. */
  isPlaying: boolean
  /** Multiplier applied by rrweb's internal timer. */
  speed: number
  setSpeed: (next: number) => void
  /** Toggles play/pause. Restarts from 0 if the session already finished. */
  togglePlay: () => void
  /** Jumps the playhead to `seconds` and pauses. */
  seek: (seconds: number) => number
  /** Updates display state from rrweb without seeking the player. */
  syncFromPlayer: (seconds: number) => boolean
}

const END_EPSILON_SECONDS = 0.01

/** Owns replay transport state while rrweb owns the playback timer. */
export function usePlayback(totalSeconds: number): Playback {
  const [time, setTime] = useState(0)
  const [isPlaying, setIsPlaying] = useState(true)
  const [speed, setSpeed] = useState<number>(DEFAULT_PLAYBACK_SPEED)

  const clamp = useCallback(
    (seconds: number) => Math.max(0, Math.min(totalSeconds, seconds)),
    [totalSeconds],
  )

  useEffect(() => {
    setTime((prev) => clamp(prev))
  }, [clamp])

  const setPlaybackSpeed = useCallback((next: number) => {
    if (PLAYBACK_SPEEDS.includes(next)) setSpeed(next)
  }, [])

  const togglePlay = useCallback(() => {
    setIsPlaying((playing) => {
      if (playing) return false
      setTime((prev) => (prev >= totalSeconds ? 0 : prev))
      return totalSeconds > 0
    })
  }, [totalSeconds])

  const seek = useCallback(
    (seconds: number) => {
      const clamped = clamp(seconds)
      setTime(clamped)
      setIsPlaying(false)
      return clamped
    },
    [clamp],
  )

  const syncFromPlayer = useCallback(
    (seconds: number) => {
      const clamped = clamp(seconds)
      const finished =
        totalSeconds > 0 && clamped >= totalSeconds - END_EPSILON_SECONDS
      if (finished) {
        setTime(totalSeconds)
        setIsPlaying(false)
        return false
      }
      setTime(clamped)
      return true
    },
    [clamp, totalSeconds],
  )

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
