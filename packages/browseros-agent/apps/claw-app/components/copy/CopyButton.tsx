/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Check, Copy } from 'lucide-react'
import { useState } from 'react'
import {
  ActionSwapBlurButton,
  type ActionSwapBlurButtonProps,
} from '@/components/motion/action-swap-blur'
import { toastApiError } from '@/modules/toast/toast'

const COPIED_FOR_MS = 1600

interface CopyButtonProps
  extends Omit<ActionSwapBlurButtonProps, 'items' | 'value' | 'onClick'> {
  text: string
  label?: string
  copiedLabel?: string
  /** Read out with the label, for example "the task id". */
  what?: string
  onCopied?: () => void
}

/** Copies text and says "Copied" in place, so no toast is needed. */
export function CopyButton({
  text,
  label = 'Copy',
  copiedLabel = 'Copied',
  what,
  onCopied,
  ...props
}: CopyButtonProps) {
  const [copied, setCopied] = useState(false)

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text)
    } catch (error) {
      toastApiError('Could not copy that', error)
      return
    }
    setCopied(true)
    onCopied?.()
    window.setTimeout(() => setCopied(false), COPIED_FOR_MS)
  }

  return (
    <ActionSwapBlurButton
      {...props}
      cycle={false}
      value={copied ? 'copied' : 'copy'}
      onClick={copy}
      items={[
        {
          id: 'copy',
          label,
          icon: <Copy className="size-[15px]" />,
          ariaLabel: what ? `${label} ${what}` : undefined,
        },
        {
          id: 'copied',
          label: copiedLabel,
          icon: <Check className="size-[15px]" />,
          ariaLabel: what ? `${copiedLabel} ${what}` : undefined,
        },
      ]}
    />
  )
}
