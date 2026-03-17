import { platform } from 'node:os'
import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'

// Meta (Cmd) on macOS, Control on everything else
const PLATFORM_MODIFIER = platform() === 'darwin' ? 4 : 2

type KeyInfo = { code: string; keyCode: number | undefined }

const KEY_MAP: Record<string, KeyInfo> = {
  Backspace: { code: 'Backspace', keyCode: 8 },
  Tab: { code: 'Tab', keyCode: 9 },
  Enter: { code: 'Enter', keyCode: 13 },
  Escape: { code: 'Escape', keyCode: 27 },
  Space: { code: 'Space', keyCode: 32 },
  ' ': { code: 'Space', keyCode: 32 },
  PageUp: { code: 'PageUp', keyCode: 33 },
  PageDown: { code: 'PageDown', keyCode: 34 },
  End: { code: 'End', keyCode: 35 },
  Home: { code: 'Home', keyCode: 36 },
  ArrowLeft: { code: 'ArrowLeft', keyCode: 37 },
  ArrowUp: { code: 'ArrowUp', keyCode: 38 },
  ArrowRight: { code: 'ArrowRight', keyCode: 39 },
  ArrowDown: { code: 'ArrowDown', keyCode: 40 },
  Insert: { code: 'Insert', keyCode: 45 },
  Delete: { code: 'Delete', keyCode: 46 },
  Shift: { code: 'ShiftLeft', keyCode: 16 },
  Control: { code: 'ControlLeft', keyCode: 17 },
  Alt: { code: 'AltLeft', keyCode: 18 },
  Meta: { code: 'MetaLeft', keyCode: 91 },
  F1: { code: 'F1', keyCode: 112 },
  F2: { code: 'F2', keyCode: 113 },
  F3: { code: 'F3', keyCode: 114 },
  F4: { code: 'F4', keyCode: 115 },
  F5: { code: 'F5', keyCode: 116 },
  F6: { code: 'F6', keyCode: 117 },
  F7: { code: 'F7', keyCode: 118 },
  F8: { code: 'F8', keyCode: 119 },
  F9: { code: 'F9', keyCode: 120 },
  F10: { code: 'F10', keyCode: 121 },
  F11: { code: 'F11', keyCode: 122 },
  F12: { code: 'F12', keyCode: 123 },
}

const KEY_ALIASES: Record<string, string> = {
  Return: 'Enter',
  Esc: 'Escape',
  Del: 'Delete',
  Ctrl: 'Control',
  Cmd: 'Meta',
  Command: 'Meta',
  Option: 'Alt',
  Left: 'ArrowLeft',
  Right: 'ArrowRight',
  Up: 'ArrowUp',
  Down: 'ArrowDown',
}

// Case-insensitive lookup: lowercase → canonical key name
const KEY_NORMALIZE = new Map<string, string>()
for (const key of Object.keys(KEY_MAP)) {
  KEY_NORMALIZE.set(key.toLowerCase(), key)
}
for (const [alias, canonical] of Object.entries(KEY_ALIASES)) {
  KEY_NORMALIZE.set(alias.toLowerCase(), canonical)
}

export function normalizeKey(key: string): string {
  if (KEY_MAP[key]) return key
  const normalized = KEY_NORMALIZE.get(key.toLowerCase())
  if (normalized) return normalized
  return key
}

const VALID_NAMED_KEYS = Object.keys(KEY_MAP).filter((k) => k.length > 1)

function isValidKey(key: string): boolean {
  if (KEY_MAP[key]) return true
  if (key.length === 1) return true
  return false
}

function validateKey(key: string): void {
  if (isValidKey(key)) return
  const aliases = Object.entries(KEY_ALIASES)
    .map(([alias, canonical]) => `${alias} → ${canonical}`)
    .join(', ')
  throw new Error(
    `Unknown key: "${key}". Valid keys: ${VALID_NAMED_KEYS.join(', ')}, ` +
      `single characters (a-z, A-Z, 0-9, symbols). Aliases: ${aliases}`,
  )
}

// Text produced by character-generating keys (matches Playwright's usKeyboardLayout)
const KEY_TEXT: Record<string, string> = {
  Enter: '\r',
  Tab: '\t',
  Space: ' ',
  ' ': ' ',
}

function getCharText(key: string): string {
  if (KEY_TEXT[key]) return KEY_TEXT[key]
  if (key.length === 1) return key
  return ''
}

const MODIFIER_BIT: Record<string, number> = {
  Alt: 1,
  Control: 2,
  Meta: 4,
  Shift: 8,
}

export function getKeyInfo(key: string): KeyInfo {
  if (KEY_MAP[key]) return KEY_MAP[key]

  if (key.length === 1) {
    if (key >= 'a' && key <= 'z')
      return {
        code: `Key${key.toUpperCase()}`,
        keyCode: key.toUpperCase().charCodeAt(0),
      }
    if (key >= 'A' && key <= 'Z')
      return { code: `Key${key}`, keyCode: key.charCodeAt(0) }
    if (key >= '0' && key <= '9')
      return { code: `Digit${key}`, keyCode: key.charCodeAt(0) }
  }

  return { code: key, keyCode: undefined }
}

export function modifierBitmask(modifiers: string[]): number {
  let mask = 0
  for (const mod of modifiers) {
    mask |= MODIFIER_BIT[mod] ?? 0
  }
  return mask
}

export async function typeText(
  session: ProtocolApi,
  text: string,
): Promise<void> {
  for (const char of text) {
    if (char === '\n') {
      await session.Input.dispatchKeyEvent({
        type: 'keyDown',
        key: 'Enter',
        code: 'Enter',
        windowsVirtualKeyCode: 13,
      })
      await session.Input.dispatchKeyEvent({
        type: 'char',
        text: '\r',
        key: 'Enter',
      })
      await session.Input.dispatchKeyEvent({
        type: 'keyUp',
        key: 'Enter',
        code: 'Enter',
        windowsVirtualKeyCode: 13,
      })
    } else {
      const info = getKeyInfo(char)
      await session.Input.dispatchKeyEvent({
        type: 'keyDown',
        key: char,
        code: info.code,
        windowsVirtualKeyCode: info.keyCode,
      })
      await session.Input.dispatchKeyEvent({
        type: 'char',
        text: char,
        key: char,
      })
      await session.Input.dispatchKeyEvent({
        type: 'keyUp',
        key: char,
        code: info.code,
        windowsVirtualKeyCode: info.keyCode,
      })
    }
  }
}

export async function clearField(session: ProtocolApi): Promise<void> {
  // Select all: Cmd+A on macOS, Ctrl+A on others
  await session.Input.dispatchKeyEvent({
    type: 'keyDown',
    key: 'a',
    code: 'KeyA',
    modifiers: PLATFORM_MODIFIER,
    windowsVirtualKeyCode: 65,
  })
  await session.Input.dispatchKeyEvent({
    type: 'keyUp',
    key: 'a',
    code: 'KeyA',
    modifiers: PLATFORM_MODIFIER,
    windowsVirtualKeyCode: 65,
  })
  // Backspace to delete selection (more reliable cross-platform than Delete)
  await session.Input.dispatchKeyEvent({
    type: 'keyDown',
    key: 'Backspace',
    code: 'Backspace',
    windowsVirtualKeyCode: 8,
  })
  await session.Input.dispatchKeyEvent({
    type: 'keyUp',
    key: 'Backspace',
    code: 'Backspace',
    windowsVirtualKeyCode: 8,
  })
}

function parseKeyCombo(input: string): {
  key: string
  modifiers: string[]
} {
  const parts: string[] = []
  let current = ''
  for (const ch of input) {
    // Only split on '+' when there's accumulated text (so literal '+' works)
    if (ch === '+' && current) {
      parts.push(current)
      current = ''
    } else {
      current += ch
    }
  }
  if (current) parts.push(current)
  if (parts.length === 0) throw new Error('Empty key input')
  const key = parts[parts.length - 1]
  return { key, modifiers: parts.slice(0, -1) }
}

export async function pressCombo(
  session: ProtocolApi,
  key: string,
): Promise<void> {
  const parsed = parseKeyCombo(key)
  const mainKey = normalizeKey(parsed.key)
  const modifiers = parsed.modifiers.map(normalizeKey)

  validateKey(mainKey)
  for (const mod of modifiers) validateKey(mod)

  const modBitmask = modifierBitmask(modifiers)

  for (const mod of modifiers) {
    const info = getKeyInfo(mod)
    await session.Input.dispatchKeyEvent({
      type: 'keyDown',
      key: mod,
      code: info.code,
      windowsVirtualKeyCode: info.keyCode,
    })
  }

  const mainInfo = getKeyInfo(mainKey)

  // Control/Alt/Meta suppress character generation (Shift does not)
  const suppressChar = modifiers.some(
    (m) => m === 'Control' || m === 'Alt' || m === 'Meta',
  )
  const text = suppressChar ? '' : getCharText(mainKey)

  // keyDown with text → Chrome generates both keydown + keypress DOM events
  // keyDown without text → Chrome generates only keydown
  await session.Input.dispatchKeyEvent({
    type: 'keyDown',
    key: mainKey,
    code: mainInfo.code,
    modifiers: modBitmask,
    windowsVirtualKeyCode: mainInfo.keyCode,
    ...(text && { text }),
  })

  await session.Input.dispatchKeyEvent({
    type: 'keyUp',
    key: mainKey,
    code: mainInfo.code,
    modifiers: modBitmask,
    windowsVirtualKeyCode: mainInfo.keyCode,
  })

  for (const mod of modifiers.reverse()) {
    const info = getKeyInfo(mod)
    await session.Input.dispatchKeyEvent({
      type: 'keyUp',
      key: mod,
      code: info.code,
      windowsVirtualKeyCode: info.keyCode,
    })
  }
}
