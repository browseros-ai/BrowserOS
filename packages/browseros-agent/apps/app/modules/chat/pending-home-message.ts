import type { FileUIPart } from 'ai'
import type { StagedAttachment } from '@/lib/attachments'

const MAX_AGE_MS = 30_000

interface PendingHomeMessage {
  token: string
  text: string
  files: FileUIPart[]
  createdAt: number
}

let pending: PendingHomeMessage | null = null
let expiration: ReturnType<typeof setTimeout> | null = null

export function stagePendingHomeMessage(input: {
  text: string
  attachments: StagedAttachment[]
}): string {
  clearPending()
  const token = crypto.randomUUID()
  pending = {
    token,
    text: input.text,
    files: input.attachments.map(toFilePart),
    createdAt: Date.now(),
  }
  expiration = setTimeout(clearPending, MAX_AGE_MS)
  return token
}

export function consumePendingHomeMessage(
  token: string | null,
): Omit<PendingHomeMessage, 'token' | 'createdAt'> | null {
  if (!pending || pending.token !== token) return null
  const message = pending
  clearPending()
  if (Date.now() - message.createdAt > MAX_AGE_MS) return null
  return { text: message.text, files: message.files }
}

function clearPending(): void {
  pending = null
  if (expiration) clearTimeout(expiration)
  expiration = null
}

function toFilePart(attachment: StagedAttachment): FileUIPart {
  const payload = attachment.payload
  if (payload.kind === 'image') {
    return {
      type: 'file',
      mediaType: payload.mediaType,
      filename: payload.name,
      url: payload.dataUrl,
    }
  }

  return {
    type: 'file',
    mediaType: payload.mediaType,
    filename: payload.name,
    url: `data:${payload.mediaType};base64,${encodeBase64(payload.text)}`,
  }
}

function encodeBase64(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let binary = ''
  for (let offset = 0; offset < bytes.length; offset += 32_768) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 32_768))
  }
  return btoa(binary)
}
