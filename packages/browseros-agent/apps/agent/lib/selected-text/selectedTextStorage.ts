import { storage } from '@wxt-dev/storage'

export interface SelectedTextData {
  text: string
  pageUrl: string
  pageTitle: string
  tabId: number
  timestamp: number
}

export const selectedTextStorage = storage.defineItem<SelectedTextData | null>(
  'local:selectedText',
  { defaultValue: null },
)
