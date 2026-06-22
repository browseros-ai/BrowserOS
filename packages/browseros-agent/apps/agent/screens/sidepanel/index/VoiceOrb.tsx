import { useEffect, useRef } from 'react'

export type VoiceOrbState = 'idle' | 'listening' | 'speaking'

interface VoiceOrbProps {
  size?: number
  state?: VoiceOrbState
  accent?: string
  className?: string
}

const POINT_COUNT = 1100
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5))

interface OrbPoint {
  x: number
  y: number
  z: number
  seed: number
}

export function VoiceOrb({
  size = 96,
  state = 'idle',
  accent = '#E8722E',
  className,
}: VoiceOrbProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const stateRef = useRef(state)
  const accentRef = useRef(accent)
  stateRef.current = state
  accentRef.current = accent

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const dpr = Math.min(window.devicePixelRatio || 1, 2)
    canvas.width = size * dpr
    canvas.height = size * dpr
    const ctx = canvas.getContext('2d')
    if (!ctx) return
    ctx.scale(dpr, dpr)

    const pts: OrbPoint[] = []
    for (let i = 0; i < POINT_COUNT; i++) {
      const y = 1 - (i / (POINT_COUNT - 1)) * 2
      const r = Math.sqrt(1 - y * y)
      const theta = GOLDEN_ANGLE * i
      pts.push({
        x: Math.cos(theta) * r,
        y,
        z: Math.sin(theta) * r,
        seed: Math.random(),
      })
    }

    let raf = 0
    let t = 0
    let amp = 0.25
    const cx = size / 2
    const cy = size / 2
    const baseR = size * 0.34

    const frame = () => {
      t += 0.01
      const st = stateRef.current
      const target = st === 'speaking' ? 0.9 : st === 'listening' ? 0.55 : 0.18
      amp += (target - amp) * 0.06

      const env =
        st === 'idle'
          ? 0
          : (0.5 + 0.5 * Math.sin(t * 2.1)) *
            (0.5 + 0.5 * Math.sin(t * 5.3 + 1)) *
            (0.5 + 0.5 * Math.sin(t * 1.3))

      ctx.clearRect(0, 0, size, size)

      const rotY = t * 0.35
      const rotX = Math.sin(t * 0.4) * 0.25
      const cosY = Math.cos(rotY)
      const sinY = Math.sin(rotY)
      const cosX = Math.cos(rotX)
      const sinX = Math.sin(rotX)

      const [ar, ag, ab] = hexToRgb(accentRef.current)
      const tint = Math.min(1, amp * (0.5 + env * 0.9))

      if (st !== 'idle') {
        const g = ctx.createRadialGradient(
          cx,
          cy,
          baseR * 0.2,
          cx,
          cy,
          baseR * 1.9,
        )
        g.addColorStop(0, `rgba(${ar},${ag},${ab},${0.1 + tint * 0.12})`)
        g.addColorStop(1, 'rgba(0,0,0,0)')
        ctx.fillStyle = g
        ctx.fillRect(0, 0, size, size)
      }

      for (let i = 0; i < POINT_COUNT; i++) {
        const p = pts[i]
        const ripple = Math.sin(t * 3 + p.seed * 12 + p.y * 4) * 0.5 + 0.5
        const disp = 1 + amp * 0.1 + env * 0.16 * ripple

        const x = p.x * disp
        const y = p.y * disp
        const z = p.z * disp
        const x1 = x * cosY - z * sinY
        const z1 = x * sinY + z * cosY
        const y1 = y * cosX - z1 * sinX
        const z2 = y * sinX + z1 * cosX

        const persp = 1 / (1.8 - z2 * 0.6)
        const sx = cx + x1 * baseR * persp
        const sy = cy + y1 * baseR * persp

        const depth = (z2 + 1) / 2
        const rad = (0.5 + depth * 1.1) * (0.8 + amp * 0.5)
        let alpha = 0.18 + depth * 0.6

        const localTint = tint * depth
        const cr = Math.round(150 + (ar - 150) * localTint)
        const cg = Math.round(150 + (ag - 150) * localTint)
        const cb = Math.round(150 + (ab - 150) * localTint)
        alpha *= 0.55 + 0.45 * depth

        ctx.beginPath()
        ctx.arc(sx, sy, rad, 0, Math.PI * 2)
        ctx.fillStyle = `rgba(${cr},${cg},${cb},${alpha})`
        ctx.fill()
      }
      raf = requestAnimationFrame(frame)
    }
    frame()
    return () => cancelAnimationFrame(raf)
  }, [size])

  return (
    <canvas
      ref={canvasRef}
      style={{ width: size, height: size, display: 'block' }}
      className={className}
      aria-hidden
    />
  )
}

function hexToRgb(hex: string): [number, number, number] {
  const m = hex.replace('#', '')
  return [
    Number.parseInt(m.slice(0, 2), 16),
    Number.parseInt(m.slice(2, 4), 16),
    Number.parseInt(m.slice(4, 6), 16),
  ]
}
