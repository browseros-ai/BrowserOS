import { createRoot, type Root } from 'react-dom/client'
import { sendSwitcherMessage } from '@/lib/messaging/switcher/switcherMessages'
import { TabSwitcherOverlay } from './TabSwitcherOverlay'
import type { LayoutMode, SwitcherState } from './types'

export class TabSwitcherController {
  private _state: SwitcherState = {
    phase: 'idle',
    tabs: [],
    selectedIndex: 0,
    thumbnails: {},
    searchQuery: '',
    layoutMode: 'strip',
  }

  private dialogEl: HTMLDialogElement | null = null
  private root: Root | null = null
  private mountPoint: HTMLDivElement | null = null

  get state(): SwitcherState {
    return this._state
  }

  private handleDialogClick = (e: MouseEvent) => {
    const path = e.composedPath()
    if (path[0] === this.dialogEl || path[0] === this.dialogEl?.firstChild) {
      this.dismiss()
    }
  }

  private getLayoutMode(tabCount: number): LayoutMode {
    return tabCount <= 7 ? 'strip' : 'grid'
  }

  async open(direction: 1 | -1) {
    const response = await sendSwitcherMessage('tab-switcher:get-tabs')
    const tabs = response.tabs
    if (tabs.length === 0) return

    const thumbnails = response.thumbnails

    const activeIdx = tabs.findIndex((t) => t.active)
    const excludeActive = activeIdx >= 0

    let selectedIndex: number
    if (direction > 0) {
      selectedIndex = excludeActive ? (activeIdx + 1) % tabs.length : 0
    } else {
      selectedIndex = excludeActive
        ? (activeIdx - 1 + tabs.length) % tabs.length
        : tabs.length - 1
    }

    this._state = {
      phase: 'open',
      tabs,
      selectedIndex,
      thumbnails,
      searchQuery: '',
      layoutMode: this.getLayoutMode(tabs.length),
    }

    if (!this.dialogEl) {
      this.createDialog()
    }

    this.render()
  }

  advance(direction: 1 | -1) {
    if (this._state.phase !== 'open') return

    const len = this._state.tabs.length
    if (len === 0) {
      this.dismiss()
      return
    }

    const next = (this._state.selectedIndex + direction + len) % len
    this._state = { ...this._state, selectedIndex: next }
    this.render()
  }

  setSearchQuery(query: string) {
    if (this._state.phase !== 'open') return
    this._state = { ...this._state, searchQuery: query }
    this.render()
  }

  async select() {
    if (this._state.phase !== 'open') return

    const tab = this._state.tabs[this._state.selectedIndex]
    if (tab?.id && tab.windowId) {
      await sendSwitcherMessage('tab-switcher:switch', {
        tabId: tab.id,
        windowId: tab.windowId,
      })
    }

    this.dismiss()
  }

  async selectTab(index: number) {
    if (this._state.phase !== 'open') return
    this._state = { ...this._state, selectedIndex: index }
    await this.select()
  }

  dismiss() {
    if (this._state.phase === 'idle') return

    this._state = {
      phase: 'idle',
      tabs: [],
      selectedIndex: 0,
      thumbnails: {},
      searchQuery: '',
      layoutMode: 'strip',
    }

    if (this.root) {
      this.root.unmount()
      this.root = null
    }
    this.mountPoint = null

    if (this.dialogEl) {
      this.dialogEl.removeEventListener('click', this.handleDialogClick)
      this.dialogEl.close()
      this.dialogEl.remove()
      this.dialogEl = null
    }
  }

  private createDialog() {
    const dialog = document.createElement('dialog')
    dialog.style.cssText = [
      'all: unset',
      'display: flex',
      'position: fixed',
      'inset: 0',
      'width: 100vw',
      'height: 100vh',
      'max-width: 100vw',
      'max-height: 100vh',
      'border: none',
      'padding: 0',
      'margin: 0',
      'background: transparent',
      'overflow: hidden',
      'align-items: center',
      'justify-content: center',
    ].join('; ')

    const backdrop = document.createElement('div')
    backdrop.style.cssText = [
      'position: fixed',
      'inset: 0',
      'background: rgba(0, 0, 0, 0.6)',
      'backdrop-filter: blur(4px)',
      'z-index: 0',
      'animation: bos-backdrop-in 0.15s ease-out',
    ].join('; ')
    dialog.appendChild(backdrop)

    const host = document.createElement('div')
    host.style.cssText = [
      'position: relative',
      'z-index: 1',
      'display: flex',
      'align-items: center',
      'justify-content: center',
    ].join('; ')
    dialog.appendChild(host)

    const shadow = host.attachShadow({ mode: 'closed' })

    const styleEl = document.createElement('style')
    styleEl.textContent = [
      '#bos-switcher-root {',
      '  all: initial;',
      '  display: flex;',
      '  align-items: center;',
      '  justify-content: center;',
      '  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;',
      '  color: #fff;',
      '}',
      '@keyframes bos-backdrop-in {',
      '  from { opacity: 0; }',
      '  to { opacity: 1; }',
      '}',
      '@keyframes bos-card-enter {',
      '  from { opacity: 0; transform: translateY(16px) scale(0.85); }',
      '  to { opacity: 1; transform: translateY(0) scale(1); }',
      '}',
      '::-webkit-scrollbar { height: 6px; }',
      '::-webkit-scrollbar-track { background: transparent; }',
      '::-webkit-scrollbar-thumb {',
      '  background: rgba(255, 255, 255, 0.2);',
      '  border-radius: 3px;',
      '}',
    ].join('\n')
    shadow.appendChild(styleEl)

    const mount = document.createElement('div')
    mount.id = 'bos-switcher-root'
    shadow.appendChild(mount)

    document.documentElement.appendChild(dialog)
    dialog.showModal()

    dialog.addEventListener('click', this.handleDialogClick)

    this.dialogEl = dialog
    this.mountPoint = mount

    if (!this.root) {
      this.root = createRoot(mount)
    }
  }

  private render() {
    if (!this.root || !this.mountPoint) return

    this.root.render(
      <TabSwitcherOverlay
        state={this._state}
        onSelectTab={(index) => this.selectTab(index)}
        onAdvance={(dir) => this.advance(dir)}
        onSelect={() => this.select()}
        onDismiss={() => this.dismiss()}
        onSearchChange={(query) => this.setSearchQuery(query)}
      />,
    )
  }
}
