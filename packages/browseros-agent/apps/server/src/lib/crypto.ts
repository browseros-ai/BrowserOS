import { createCipheriv, createDecipheriv, randomBytes, scryptSync } from 'node:crypto'

const ALGORITHM = 'aes-256-gcm'
const IV_LENGTH = 12

// In production, BROWSEROS_ENCRYPTION_KEY should be set.
// If not, we derive a key from a fixed salt for basic protection against casual inspection.
const ENCRYPTION_KEY_RAW = process.env.BROWSEROS_ENCRYPTION_KEY || 'default-browseros-internal-key-change-me'
const SALT = 'browseros-encryption-salt'
const KEY = scryptSync(ENCRYPTION_KEY_RAW, SALT, 32)

export function encrypt(text: string): string {
  const iv = randomBytes(IV_LENGTH)
  const cipher = createCipheriv(ALGORITHM, KEY, iv)
  
  let encrypted = cipher.update(text, 'utf8', 'hex')
  encrypted += cipher.final('hex')
  
  const authTag = cipher.getAuthTag().toString('hex')
  
  // Format: iv:authTag:encryptedData
  return `${iv.toString('hex')}:${authTag}:${encrypted}`
}

export function decrypt(encryptedData: string): string {
  const [ivHex, authTagHex, encrypted] = encryptedData.split(':')
  if (!ivHex || !authTagHex || !encrypted) {
    throw new Error('Invalid encrypted data format')
  }

  const iv = Buffer.from(ivHex, 'hex')
  const authTag = Buffer.from(authTagHex, 'hex')
  const decipher = createDecipheriv(ALGORITHM, KEY, iv)
  
  decipher.setAuthTag(authTag)
  
  let decrypted = decipher.update(encrypted, 'hex', 'utf8')
  decrypted += decipher.final('utf8')
  
  return decrypted
}

/**
 * Safely decrypts data, returning original string if decryption fails 
 * (useful for migrating existing plaintext data)
 */
export function tryDecrypt(data: string): string {
  try {
    return decrypt(data)
  } catch {
    return data
  }
}
