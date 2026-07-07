import { describe, expect, it } from 'bun:test'
import {
  Children,
  isValidElement,
  type ReactElement,
  type ReactNode,
} from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { WelcomeStep } from './WelcomeStep'

type ClickableElement = ReactElement<{
  children?: ReactNode
  onClick?: () => void
}>

function getText(node: ReactNode): string {
  if (typeof node === 'string' || typeof node === 'number') {
    return String(node)
  }
  if (!isValidElement(node)) {
    return Children.toArray(node).map(getText).join('')
  }
  const props = node.props as { children?: ReactNode }
  return getText(props.children)
}

function findClickableByText(
  node: ReactNode,
  label: string,
): ClickableElement | null {
  for (const child of Children.toArray(node)) {
    if (!isValidElement(child)) continue
    const props = child.props as {
      children?: ReactNode
      onClick?: () => void
    }
    if (
      typeof props.onClick === 'function' &&
      getText(props.children).includes(label)
    ) {
      return child as ClickableElement
    }
    const nested = findClickableByText(props.children, label)
    if (nested) return nested
  }
  return null
}

describe('WelcomeStep', () => {
  it('renders the setup CTA', () => {
    const html = renderToStaticMarkup(
      <WelcomeStep onPrimary={() => undefined} onSkip={() => undefined} />,
    )

    expect(html).toContain('The browser your agents')
    expect(html).toContain('Start setup')
  })

  it('wires the reconnect CTA to skip setup', () => {
    let skipped = false
    const tree = WelcomeStep({
      onPrimary: () => undefined,
      onSkip: () => {
        skipped = true
      },
    })

    const reconnectButton = findClickableByText(tree, 'Reconnect.')
    expect(reconnectButton).not.toBeNull()

    reconnectButton?.props.onClick?.()
    expect(skipped).toBe(true)
  })
})
