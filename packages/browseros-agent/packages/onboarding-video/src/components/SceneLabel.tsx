/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Small caption placed above a surface in the demo. Used for
 * "where your work happens" and "you are here" style annotations.
 */

import { fonts } from '../fonts'
import { palette } from '../palette'

interface SceneLabelProps {
  text: string
  align?: 'left' | 'center'
  opacity?: number
  style?: React.CSSProperties
}

export function SceneLabel({
  text,
  align = 'left',
  opacity = 1,
  style,
}: SceneLabelProps) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        fontFamily: fonts.mono,
        fontSize: 12,
        letterSpacing: 2,
        color: palette.ink3,
        textTransform: 'uppercase',
        justifyContent: align === 'center' ? 'center' : 'flex-start',
        opacity,
        ...style,
      }}
    >
      <span
        style={{
          width: 6,
          height: 6,
          borderRadius: 999,
          background: palette.accent,
        }}
      />
      {text}
    </div>
  )
}
