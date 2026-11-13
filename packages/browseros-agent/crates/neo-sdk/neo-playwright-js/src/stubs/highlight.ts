/**
 * Copyright (c) Microsoft Corporation.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
// Screenshot-only subset of Playwright's Highlight. The selector engine owns box measurement;
// the popover and closed shadow root keep masks above page content without intercepting input.
// Inspector, recorder and screencast entry points remain build-time stand-ins.
import type { InjectedScript } from '../../../neo-playwright/third_party/playwright/packages/injected/src/injectedScript'

export class Highlight {
  // biome-ignore lint/suspicious/noExplicitAny: upstream's unused inspector methods need their declared return types.
  readonly [method: string]: (...args: any[]) => any
  readonly #pane: HTMLElement
  readonly #shadow: ShadowRoot
  readonly #style: HTMLStyleElement
  readonly #entries: { element: Element; color: string }[] = []

  readonly #injected: InjectedScript

  constructor(injected: InjectedScript) {
    this.#injected = injected
    this.#pane = injected.document.createElement('x-pw-glass')
    this.#pane.setAttribute('popover', 'manual')
    this.#pane.style.cssText =
      'inset:0;width:100%;height:100%;max-width:none;max-height:none;padding:0;margin:0;border:none;overflow:visible;pointer-events:none;display:flex;background-color:transparent'
    this.#shadow = this.#pane.attachShadow({ mode: 'closed' })
    // Host defaults affect CSS colors such as currentColor and light-dark(), even when the
    // page itself chooses another text color or color scheme. Keep upstream's declarations.
    this.#style = injected.document.createElement('style')
    this.#style.textContent =
      ':host{font-size:13px;font-family:system-ui,"Ubuntu","Droid Sans",sans-serif;color:#333;color-scheme:light}'
    this.#shadow.appendChild(this.#style)
  }

  install() {
    const root = this.#injected.document.documentElement
    if (!root) return
    if (!root.contains(this.#pane) || this.#pane.nextElementSibling)
      root.appendChild(this.#pane)
    this.#pane.hidePopover()
    this.#pane.showPopover()
  }

  uninstall() {
    this.#pane.remove()
    this.#entries.length = 0
  }

  addMaskedElements(elements: Element[], color: string) {
    for (const element of elements) this.#entries.push({ element, color })
    this.#shadow.replaceChildren(this.#style)
    const highlights = this.#entries.map(() => {
      const highlight = this.#injected.document.createElement('x-pw-highlight')
      highlight.style.cssText =
        'position:absolute;top:0;left:0;width:0;height:0'
      this.#shadow.appendChild(highlight)
      return highlight
    })
    // Match upstream's write/read/write phases: one layout measures all target border boxes.
    const boxes = this.#entries.map(({ element }) =>
      element.getBoundingClientRect(),
    )
    for (const [index, highlight] of highlights.entries()) {
      const box = boxes[index],
        entry = this.#entries[index]
      if (!box || !entry) continue
      highlight.style.backgroundColor = entry.color
      highlight.style.left = `${box.x}px`
      highlight.style.top = `${box.y}px`
      highlight.style.width = `${box.width}px`
      highlight.style.height = `${box.height}px`
      highlight.style.display = 'block'
    }
  }
}
