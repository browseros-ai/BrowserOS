// Generated from contracts/server-api/openapi.yaml. Do not edit.
import type { UiMessageChunk } from '@browseros/server-api'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
/** Validates stream payloads without dropping provider metadata or extension fields. */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: generated cases validate the complete chunk union.
export function isUiMessageChunk(value: unknown): value is UiMessageChunk {
  if (!isRecord(value)) return false
  switch (value.type) {
    case 'start':
      return (
        isRecord(value) &&
        'type' in value &&
        ['start'].some((allowed0) => allowed0 === value.type) &&
        (!('messageId' in value) || typeof value.messageId === 'string') &&
        (!('messageMetadata' in value) || true)
      )
    case 'start-step':
      return (
        isRecord(value) &&
        'type' in value &&
        ['start-step'].some((allowed1) => allowed1 === value.type)
      )
    case 'finish-step':
      return (
        isRecord(value) &&
        'type' in value &&
        ['finish-step'].some((allowed2) => allowed2 === value.type)
      )
    case 'finish':
      return (
        isRecord(value) &&
        'type' in value &&
        ['finish'].some((allowed3) => allowed3 === value.type) &&
        (!('finishReason' in value) ||
          [
            'stop',
            'length',
            'content-filter',
            'tool-calls',
            'error',
            'other',
          ].some((allowed4) => allowed4 === value.finishReason)) &&
        (!('messageMetadata' in value) || true)
      )
    case 'abort':
      return (
        isRecord(value) &&
        'type' in value &&
        ['abort'].some((allowed5) => allowed5 === value.type) &&
        (!('reason' in value) || typeof value.reason === 'string')
      )
    case 'error':
      return (
        isRecord(value) &&
        'type' in value &&
        ['error'].some((allowed6) => allowed6 === value.type) &&
        'errorText' in value &&
        typeof value.errorText === 'string'
      )
    case 'text-start':
      return (
        isRecord(value) &&
        'type' in value &&
        ['text-start'].some((allowed7) => allowed7 === value.type) &&
        (!('providerMetadata' in value) ||
          (isRecord(value.providerMetadata) &&
            Object.values(value.providerMetadata).every((item8) =>
              isRecord(item8),
            ))) &&
        'id' in value &&
        typeof value.id === 'string'
      )
    case 'text-delta':
      return (
        isRecord(value) &&
        'type' in value &&
        ['text-delta'].some((allowed9) => allowed9 === value.type) &&
        (!('providerMetadata' in value) ||
          (isRecord(value.providerMetadata) &&
            Object.values(value.providerMetadata).every((item10) =>
              isRecord(item10),
            ))) &&
        'id' in value &&
        typeof value.id === 'string' &&
        'delta' in value &&
        typeof value.delta === 'string'
      )
    case 'text-end':
      return (
        isRecord(value) &&
        'type' in value &&
        ['text-end'].some((allowed11) => allowed11 === value.type) &&
        (!('providerMetadata' in value) ||
          (isRecord(value.providerMetadata) &&
            Object.values(value.providerMetadata).every((item12) =>
              isRecord(item12),
            ))) &&
        'id' in value &&
        typeof value.id === 'string'
      )
    case 'reasoning-start':
      return (
        isRecord(value) &&
        'type' in value &&
        ['reasoning-start'].some((allowed13) => allowed13 === value.type) &&
        (!('providerMetadata' in value) ||
          (isRecord(value.providerMetadata) &&
            Object.values(value.providerMetadata).every((item14) =>
              isRecord(item14),
            ))) &&
        'id' in value &&
        typeof value.id === 'string'
      )
    case 'reasoning-delta':
      return (
        isRecord(value) &&
        'type' in value &&
        ['reasoning-delta'].some((allowed15) => allowed15 === value.type) &&
        (!('providerMetadata' in value) ||
          (isRecord(value.providerMetadata) &&
            Object.values(value.providerMetadata).every((item16) =>
              isRecord(item16),
            ))) &&
        'id' in value &&
        typeof value.id === 'string' &&
        'delta' in value &&
        typeof value.delta === 'string'
      )
    case 'reasoning-end':
      return (
        isRecord(value) &&
        'type' in value &&
        ['reasoning-end'].some((allowed17) => allowed17 === value.type) &&
        (!('providerMetadata' in value) ||
          (isRecord(value.providerMetadata) &&
            Object.values(value.providerMetadata).every((item18) =>
              isRecord(item18),
            ))) &&
        'id' in value &&
        typeof value.id === 'string'
      )
    case 'tool-input-start':
      return (
        isRecord(value) &&
        'type' in value &&
        ['tool-input-start'].some((allowed19) => allowed19 === value.type) &&
        (!('providerMetadata' in value) ||
          (isRecord(value.providerMetadata) &&
            Object.values(value.providerMetadata).every((item20) =>
              isRecord(item20),
            ))) &&
        (!('providerExecuted' in value) ||
          typeof value.providerExecuted === 'boolean') &&
        (!('toolMetadata' in value) || isRecord(value.toolMetadata)) &&
        (!('dynamic' in value) || typeof value.dynamic === 'boolean') &&
        (!('title' in value) || typeof value.title === 'string') &&
        'toolCallId' in value &&
        typeof value.toolCallId === 'string' &&
        'toolName' in value &&
        typeof value.toolName === 'string'
      )
    case 'tool-input-delta':
      return (
        isRecord(value) &&
        'type' in value &&
        ['tool-input-delta'].some((allowed21) => allowed21 === value.type) &&
        'toolCallId' in value &&
        typeof value.toolCallId === 'string' &&
        'inputTextDelta' in value &&
        typeof value.inputTextDelta === 'string'
      )
    case 'tool-input-available':
      return (
        isRecord(value) &&
        'type' in value &&
        ['tool-input-available'].some(
          (allowed22) => allowed22 === value.type,
        ) &&
        (!('providerMetadata' in value) ||
          (isRecord(value.providerMetadata) &&
            Object.values(value.providerMetadata).every((item23) =>
              isRecord(item23),
            ))) &&
        (!('providerExecuted' in value) ||
          typeof value.providerExecuted === 'boolean') &&
        (!('toolMetadata' in value) || isRecord(value.toolMetadata)) &&
        (!('dynamic' in value) || typeof value.dynamic === 'boolean') &&
        (!('title' in value) || typeof value.title === 'string') &&
        'toolCallId' in value &&
        typeof value.toolCallId === 'string' &&
        'toolName' in value &&
        typeof value.toolName === 'string' &&
        'input' in value &&
        true
      )
    case 'tool-input-error':
      return (
        isRecord(value) &&
        'type' in value &&
        ['tool-input-error'].some((allowed24) => allowed24 === value.type) &&
        (!('providerMetadata' in value) ||
          (isRecord(value.providerMetadata) &&
            Object.values(value.providerMetadata).every((item25) =>
              isRecord(item25),
            ))) &&
        (!('providerExecuted' in value) ||
          typeof value.providerExecuted === 'boolean') &&
        (!('toolMetadata' in value) || isRecord(value.toolMetadata)) &&
        (!('dynamic' in value) || typeof value.dynamic === 'boolean') &&
        (!('title' in value) || typeof value.title === 'string') &&
        'toolCallId' in value &&
        typeof value.toolCallId === 'string' &&
        'toolName' in value &&
        typeof value.toolName === 'string' &&
        'input' in value &&
        true &&
        'errorText' in value &&
        typeof value.errorText === 'string'
      )
    case 'tool-output-available':
      return (
        isRecord(value) &&
        'type' in value &&
        ['tool-output-available'].some(
          (allowed26) => allowed26 === value.type,
        ) &&
        (!('providerMetadata' in value) ||
          (isRecord(value.providerMetadata) &&
            Object.values(value.providerMetadata).every((item27) =>
              isRecord(item27),
            ))) &&
        (!('providerExecuted' in value) ||
          typeof value.providerExecuted === 'boolean') &&
        (!('toolMetadata' in value) || isRecord(value.toolMetadata)) &&
        (!('dynamic' in value) || typeof value.dynamic === 'boolean') &&
        (!('title' in value) || typeof value.title === 'string') &&
        'toolCallId' in value &&
        typeof value.toolCallId === 'string' &&
        'output' in value &&
        true &&
        (!('preliminary' in value) || typeof value.preliminary === 'boolean')
      )
    case 'tool-output-error':
      return (
        isRecord(value) &&
        'type' in value &&
        ['tool-output-error'].some((allowed28) => allowed28 === value.type) &&
        (!('providerMetadata' in value) ||
          (isRecord(value.providerMetadata) &&
            Object.values(value.providerMetadata).every((item29) =>
              isRecord(item29),
            ))) &&
        (!('providerExecuted' in value) ||
          typeof value.providerExecuted === 'boolean') &&
        (!('toolMetadata' in value) || isRecord(value.toolMetadata)) &&
        (!('dynamic' in value) || typeof value.dynamic === 'boolean') &&
        (!('title' in value) || typeof value.title === 'string') &&
        'toolCallId' in value &&
        typeof value.toolCallId === 'string' &&
        'errorText' in value &&
        typeof value.errorText === 'string'
      )
    case 'source-url':
      return (
        isRecord(value) &&
        'type' in value &&
        ['source-url'].some((allowed30) => allowed30 === value.type) &&
        (!('providerMetadata' in value) ||
          (isRecord(value.providerMetadata) &&
            Object.values(value.providerMetadata).every((item31) =>
              isRecord(item31),
            ))) &&
        'sourceId' in value &&
        typeof value.sourceId === 'string' &&
        'url' in value &&
        typeof value.url === 'string' &&
        (!('title' in value) || typeof value.title === 'string')
      )
    case 'file':
      return (
        isRecord(value) &&
        'type' in value &&
        ['file'].some((allowed32) => allowed32 === value.type) &&
        (!('providerMetadata' in value) ||
          (isRecord(value.providerMetadata) &&
            Object.values(value.providerMetadata).every((item33) =>
              isRecord(item33),
            ))) &&
        'url' in value &&
        typeof value.url === 'string' &&
        'mediaType' in value &&
        typeof value.mediaType === 'string'
      )
    default:
      return false
  }
}
