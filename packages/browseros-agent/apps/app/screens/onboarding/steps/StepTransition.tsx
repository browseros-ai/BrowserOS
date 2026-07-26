import { motion } from 'motion/react'
import type { FC, PropsWithChildren } from 'react'

export type StepDirection = -1 | 1

export interface StepTransitionProps {
  direction: StepDirection
}

const variants = {
  enter: (direction: StepDirection) => {
    return {
      x: direction > 0 ? 1000 : -1000,
      opacity: 0,
    }
  },
  center: {
    x: 0,
    opacity: 1,
  },
  exit: (direction: StepDirection) => {
    return {
      x: direction < 0 ? 1000 : -1000,
      opacity: 0,
    }
  },
}

export const StepTransition: FC<PropsWithChildren<StepTransitionProps>> = ({
  children,
  direction,
}) => {
  return (
    <motion.div
      variants={variants}
      initial="enter"
      animate="center"
      className="absolute inset-0 h-[550px]"
      exit="exit"
      custom={direction}
      transition={{
        x: { type: 'spring', stiffness: 300, damping: 30 },
        opacity: { duration: 0.2 },
      }}
    >
      {children}
    </motion.div>
  )
}
