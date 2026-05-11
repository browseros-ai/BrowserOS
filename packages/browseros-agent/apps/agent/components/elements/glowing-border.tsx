import {
  motion,
  useAnimationFrame,
  useMotionTemplate,
  useMotionValue,
  useTransform,
} from 'motion/react'
import React, { type FC, type SVGProps, useRef } from 'react'

interface GlowingBorderProps extends SVGProps<SVGSVGElement> {
  duration?: number
  rx?: string
  ry?: string
  delay?: number
}

/**
 * @public
 */
export const GlowingBorder: FC<GlowingBorderProps> = ({
  children,
  duration = 3000,
  rx,
  ry,
  delay = 0,
  ...otherProps
}) => {
  const pathRef = useRef<SVGRectElement>(null)
  const progress = useMotionValue<number>(-1) // -1: pre-animation, 0-1: animation
  const startTime = useRef<number | null>(null)
  const isFinished = useRef(false)
  const startOffset = useRef<number>(0)
  const lengthRef = useRef<number>(0)

  useAnimationFrame((time) => {
    if (isFinished.current) return

    const length = pathRef.current?.getTotalLength()
    if (!length) return
    lengthRef.current = length

    if (startTime.current === null && pathRef.current) {
      startTime.current = time
      const bbox = pathRef.current.getBBox()
      const width = bbox.width
      const rxPercentage = rx ? parseFloat(rx) / 100 : 0
      const rxPixels = rxPercentage * width
      startOffset.current = length / 2 + (width / 2 - rxPixels)
    }

    const elapsedTimeSinceMount = time - (startTime.current as number)
    if (elapsedTimeSinceMount < delay) {
      return
    }
    const elapsedTime = elapsedTimeSinceMount - delay

    const effectiveDuration = duration * 0.5 // each travels 50% of path

    if (elapsedTime >= effectiveDuration) {
      progress.set(1)
      isFinished.current = true
      return
    }

    progress.set(elapsedTime / effectiveDuration)
  })

  const distance = useTransform(progress, (p) => p * lengthRef.current * 0.5)
  const opacity = useTransform(progress, [-1, 0, 0.9, 1], [0, 1, 1, 0])

  // Circle 1 (moves forward)
  const progress1 = useTransform(distance, (d) => startOffset.current + d)
  const x1 = useTransform(progress1, (val) => {
    const l = lengthRef.current
    if (!l || !pathRef.current) return 0
    return pathRef.current.getPointAtLength(val % l).x
  })
  const y1 = useTransform(progress1, (val) => {
    const l = lengthRef.current
    if (!l || !pathRef.current) return 0
    return pathRef.current.getPointAtLength(val % l).y
  })
  const transform1 = useMotionTemplate`translateX(${x1}px) translateY(${y1}px) translateX(-50%) translateY(-50%)`

  // Circle 2 (moves backward)
  const progress2 = useTransform(distance, (d) => startOffset.current - d)
  const x2 = useTransform(progress2, (val) => {
    const l = lengthRef.current
    if (!l || !pathRef.current) return 0
    return pathRef.current.getPointAtLength(((val % l) + l) % l).x
  })
  const y2 = useTransform(progress2, (val) => {
    const l = lengthRef.current
    if (!l || !pathRef.current) return 0
    return pathRef.current.getPointAtLength(((val % l) + l) % l).y
  })
  const transform2 = useMotionTemplate`translateX(${x2}px) translateY(${y2}px) translateX(-50%) translateY(-50%)`

  return (
    <>
      {/* biome-ignore lint/a11y/noSvgWithoutTitle: SVG Purely for cosmetic purpose */}
      <svg
        xmlns="http://www.w3.org/2000/svg"
        preserveAspectRatio="none"
        className="absolute h-full w-full"
        width="100%"
        height="100%"
        {...otherProps}
      >
        <rect
          fill="none"
          width="100%"
          height="100%"
          rx={rx}
          ry={ry}
          ref={pathRef}
        />
      </svg>
      <motion.div
        style={{
          position: 'absolute',
          top: 0,
          left: 0,
          display: 'inline-block',
          transform: transform1,
          opacity: opacity,
        }}
      >
        {children}
      </motion.div>
      <motion.div
        style={{
          position: 'absolute',
          top: 0,
          left: 0,
          display: 'inline-block',
          transform: transform2,
          opacity: opacity,
        }}
      >
        {React.cloneElement(children as React.ReactElement)}
      </motion.div>
    </>
  )
}

/**
 * @public
 */
export const GlowingElement: FC = () => (
  <div className="h-28 w-28 bg-[radial-gradient(var(--accent-orange)_40%,transparent_60%)] opacity-[0.8] sm:h-32 sm:w-32" />
)
