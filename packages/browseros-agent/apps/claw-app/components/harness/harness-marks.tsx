/**
 * Per-harness brand marks for the new-agent wizard and any other
 * surface that renders a harness tile. Each mark wraps a brand SVG
 * installed from the `@svgl` shadcn registry (see `components.json`).
 * Mirror of `apps/app/screens/mcp-settings/agent-marks.tsx`.
 *
 * Brand marks choose their own colors, including light/dark variants.
 * Consumer className should only provide sizing.
 */

import type { FC, SVGProps } from 'react'
import { AnthropicBlack } from '@/components/ui/svgs/anthropicBlack'
import { AnthropicWhite } from '@/components/ui/svgs/anthropicWhite'
import { Antigravity } from '@/components/ui/svgs/antigravity'
import { CodexDark } from '@/components/ui/svgs/codexDark'
import { CodexLight } from '@/components/ui/svgs/codexLight'
import { CursorDark } from '@/components/ui/svgs/cursorDark'
import { CursorLight } from '@/components/ui/svgs/cursorLight'
import { Opencode } from '@/components/ui/svgs/opencode'
import { OpencodeDark } from '@/components/ui/svgs/opencodeDark'
import { Vscode } from '@/components/ui/svgs/vscode'
import { ZedLogo } from '@/components/ui/svgs/zedLogo'
import { cn } from '@/lib/utils'

export type HarnessMarkProps = SVGProps<SVGSVGElement>

export const ClaudeCodeMark: FC<HarnessMarkProps> = ({
  className,
  ...props
}) => (
  <>
    <AnthropicBlack
      aria-hidden
      className={cn(className, 'dark:hidden')}
      {...props}
    />
    <AnthropicWhite
      aria-hidden
      className={cn(className, 'hidden dark:block')}
      {...props}
    />
  </>
)

export const CursorMark: FC<HarnessMarkProps> = ({ className, ...props }) => (
  <>
    <CursorLight
      aria-hidden
      className={cn(className, 'fill-[#111] dark:hidden')}
      {...props}
    />
    <CursorDark
      aria-hidden
      className={cn(className, 'hidden fill-white dark:block')}
      {...props}
    />
  </>
)

export const VSCodeMark: FC<HarnessMarkProps> = (props) => (
  <Vscode aria-hidden {...props} />
)

export const CodexMark: FC<HarnessMarkProps> = ({ className, ...props }) => (
  <>
    <CodexLight
      aria-hidden
      className={cn(className, 'dark:hidden')}
      {...props}
    />
    <CodexDark
      aria-hidden
      className={cn(className, 'hidden dark:block')}
      {...props}
    />
  </>
)

export const ZedMark: FC<HarnessMarkProps> = (props) => (
  <ZedLogo aria-hidden {...props} />
)

export const OpenCodeMark: FC<HarnessMarkProps> = ({ className, ...props }) => (
  <>
    <Opencode aria-hidden className={cn(className, 'dark:hidden')} {...props} />
    <OpencodeDark
      aria-hidden
      className={cn(className, 'hidden dark:block')}
      {...props}
    />
  </>
)

export const AntigravityMark: FC<HarnessMarkProps> = (props) => (
  <Antigravity aria-hidden {...props} />
)
