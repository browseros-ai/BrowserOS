import type { FC } from 'react'

export const NewTabFocusGrid: FC = () => {
  return (
    <div className="pointer-events-none absolute inset-0 overflow-hidden">
      <div className="absolute inset-0 bg-grid-pattern"></div>
      <div className="absolute inset-0 bg-gradient-radial-focus"></div>
    </div>
  )
}
