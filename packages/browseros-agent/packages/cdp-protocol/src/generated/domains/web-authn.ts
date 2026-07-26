// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

// ══ Types ══

export type AuthenticatorId = string

export type AuthenticatorProtocol = 'u2f' | 'ctap2'

export type Ctap2Version = 'ctap2_0' | 'ctap2_1' | 'ctap2_2'

export type AuthenticatorTransport =
  | 'usb'
  | 'nfc'
  | 'ble'
  | 'cable'
  | 'internal'

export interface VirtualAuthenticatorOptions {
  protocol: AuthenticatorProtocol
  ctap2Version?: Ctap2Version
  transport: AuthenticatorTransport
  hasResidentKey?: boolean
  hasUserVerification?: boolean
  hasLargeBlob?: boolean
  hasCredBlob?: boolean
  hasMinPinLength?: boolean
  hasPrf?: boolean
  hasHmacSecret?: boolean
  hasHmacSecretMc?: boolean
  automaticPresenceSimulation?: boolean
  isUserVerified?: boolean
  defaultBackupEligibility?: boolean
  defaultBackupState?: boolean
}

export interface Credential {
  credentialId: string
  isResidentCredential: boolean
  rpId?: string
  privateKey: string
  userHandle?: string
  signCount: number
  largeBlob?: string
  backupEligibility?: boolean
  backupState?: boolean
  userName?: string
  userDisplayName?: string
}

// ══ Commands ══

export interface EnableParams {
  enableUI?: boolean
}

export interface AddVirtualAuthenticatorParams {
  options: VirtualAuthenticatorOptions
}

export interface AddVirtualAuthenticatorResult {
  authenticatorId: AuthenticatorId
}

export interface SetResponseOverrideBitsParams {
  authenticatorId: AuthenticatorId
  isBogusSignature?: boolean
  isBadUV?: boolean
  isBadUP?: boolean
}

export interface RemoveVirtualAuthenticatorParams {
  authenticatorId: AuthenticatorId
}

export interface AddCredentialParams {
  authenticatorId: AuthenticatorId
  credential: Credential
}

export interface GetCredentialParams {
  authenticatorId: AuthenticatorId
  credentialId: string
}

export interface GetCredentialResult {
  credential: Credential
}

export interface GetCredentialsParams {
  authenticatorId: AuthenticatorId
}

export interface GetCredentialsResult {
  credentials: Credential[]
}

export interface RemoveCredentialParams {
  authenticatorId: AuthenticatorId
  credentialId: string
}

export interface ClearCredentialsParams {
  authenticatorId: AuthenticatorId
}

export interface SetUserVerifiedParams {
  authenticatorId: AuthenticatorId
  isUserVerified: boolean
}

export interface SetAutomaticPresenceSimulationParams {
  authenticatorId: AuthenticatorId
  enabled: boolean
}

export interface SetCredentialPropertiesParams {
  authenticatorId: AuthenticatorId
  credentialId: string
  backupEligibility?: boolean
  backupState?: boolean
}

// ══ Events ══

export interface CredentialAddedEvent {
  authenticatorId: AuthenticatorId
  credential: Credential
}

export interface CredentialDeletedEvent {
  authenticatorId: AuthenticatorId
  credentialId: string
}

export interface CredentialUpdatedEvent {
  authenticatorId: AuthenticatorId
  credential: Credential
}

export interface CredentialAssertedEvent {
  authenticatorId: AuthenticatorId
  credential: Credential
}
