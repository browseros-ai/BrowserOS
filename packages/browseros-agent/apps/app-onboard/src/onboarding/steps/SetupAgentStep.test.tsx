import { describe, expect, it } from 'bun:test'
import {
  Children,
  isValidElement,
  type ReactElement,
  type ReactNode,
} from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { SetupAgentStep } from './SetupAgentStep'

type ClickableElement = ReactElement<{
  children?: ReactNode
  onClick?: () => void
}>

function getText(node: ReactNode): string {
  if (typeof node === 'string' || typeof node === 'number') return String(node)
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
    const props = child.props as { children?: ReactNode; onClick?: () => void }
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

describe('SetupAgentStep', () => {
  it('renders the provider and coding-agent clusters with brand marks', () => {
    const html = renderToStaticMarkup(
      <SetupAgentStep onSetup={() => undefined} onLater={() => undefined} />,
    )

    expect(html).toContain('Set up your')
    expect(html).toContain('Any LLM provider')
    expect(html).toContain('Or a coding agent you already use')
    expect(html).toContain('+40')
    expect(html).toContain('Set up my agent')
    // Every chip is a brand SVG; there should be many.
    expect((html.match(/<svg/g) ?? []).length).toBeGreaterThan(10)
  })

  it('wires the primary CTA to setup', () => {
    let setup = false
    const tree = SetupAgentStep({
      onSetup: () => {
        setup = true
      },
      onLater: () => undefined,
    })

    const button = findClickableByText(tree, 'Set up my agent')
    expect(button).not.toBeNull()

    button?.props.onClick?.()
    expect(setup).toBe(true)
  })
})
