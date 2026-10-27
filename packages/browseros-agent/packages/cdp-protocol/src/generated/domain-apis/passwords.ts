// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  AddCredentialParams,
  AddCredentialResult,
  CancelOperationParams,
  FillCredentialParams,
  FillCredentialResult,
  GetContextResult,
  ListCredentialsParams,
  ListCredentialsResult,
} from '../domains/passwords'

export interface PasswordsApi {
  // ── Commands ──

  getContext(): Promise<GetContextResult>
  listCredentials(params: ListCredentialsParams): Promise<ListCredentialsResult>
  addCredential(params: AddCredentialParams): Promise<AddCredentialResult>
  fillCredential(params: FillCredentialParams): Promise<FillCredentialResult>
  cancelOperation(params: CancelOperationParams): Promise<void>
}
