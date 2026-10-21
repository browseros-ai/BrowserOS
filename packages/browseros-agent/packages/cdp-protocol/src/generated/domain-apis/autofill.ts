// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  AddCredentialParams,
  AddCredentialResult,
  AddressFormFilledEvent,
  CancelCredentialOperationParams,
  FillCredentialParams,
  FillCredentialResult,
  GetCredentialContextResult,
  ListCredentialsParams,
  ListCredentialsResult,
  SetAddressesParams,
  TriggerParams,
} from '../domains/autofill'

export interface AutofillApi {
  // ── Commands ──

  trigger(params: TriggerParams): Promise<void>
  setAddresses(params: SetAddressesParams): Promise<void>
  disable(): Promise<void>
  enable(): Promise<void>
  getCredentialContext(): Promise<GetCredentialContextResult>
  listCredentials(params: ListCredentialsParams): Promise<ListCredentialsResult>
  addCredential(params: AddCredentialParams): Promise<AddCredentialResult>
  fillCredential(params: FillCredentialParams): Promise<FillCredentialResult>
  cancelCredentialOperation(
    params: CancelCredentialOperationParams,
  ): Promise<void>

  // ── Events ──

  on(
    event: 'addressFormFilled',
    handler: (params: AddressFormFilledEvent) => void,
  ): () => void
}
