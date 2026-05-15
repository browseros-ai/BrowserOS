/**
 * 翻譯 Hook - 提供多語言支持
 * 支援繁體中文（香港）與英文
 */

import { useState, useEffect, useCallback } from 'react'
import { zhHK, type Translation } from './zh-HK'

export type Locale = 'en-US' | 'zh-HK'

// 英文翻譯（預設為空，使用英文原文）
const enUS: Partial<Translation> = {}

const translations: Record<Locale, Partial<Translation>> = {
  'en-US': enUS,
  'zh-HK': zhHK,
}

// 從 localStorage 或瀏覽器設定獲取預設語言
function getPreferredLocale(): Locale {
  if (typeof window === 'undefined') return 'en-US'
  
  try {
    const saved = localStorage.getItem('browseros-locale') as Locale | null
    if (saved && translations[saved]) return saved
    
    const browserLang = navigator.language
    if (browserLang.startsWith('zh-HK') || browserLang.startsWith('zh-TW')) {
      return 'zh-HK'
    }
  } catch {
    // Ignore errors
  }
  
  return 'en-US'
}

export interface UseTranslationReturn {
  /** 翻譯函數 */
  t: (key: keyof Translation) => string
  /** 當前語言 */
  locale: Locale
  /** 切換語言 */
  setLocale: (locale: Locale) => void
  /** 是否為中文 */
  isZhHK: boolean
}

/**
 * 使用翻譯的 Hook
 * 
 * @example
 * ```tsx
 * function MyComponent() {
 *   const { t, locale, setLocale } = useTranslation()
 *   
 *   return (
 *     <div>
 *       <h1>{t('welcomeToBrowserOS')}</h1>
 *       <button onClick={() => setLocale(locale === 'zh-HK' ? 'en-US' : 'zh-HK')}>
 *         切換語言
 *       </button>
 *     </div>
 *   )
 * }
 * ```
 */
export function useTranslation(): UseTranslationReturn {
  const [locale, setLocaleState] = useState<Locale>(getPreferredLocale)
  
  const setLocale = useCallback((newLocale: Locale) => {
    setLocaleState(newLocale)
    try {
      localStorage.setItem('browseros-locale', newLocale)
    } catch {
      // Ignore storage errors
    }
  }, [])
  
  const t = useCallback((key: keyof Translation): string => {
    const translation = translations[locale]?.[key]
    
    // 如果是中文且有翻譯，返回翻譯；否則返回英文原文（key 的駝峰轉空格）
    if (locale === 'zh-HK' && translation) {
      return translation
    }
    
    // 英文模式或沒有翻譯時，將 key 轉換為可讀的英文
    return key.replace(/([A-Z])/g, ' $1').replace(/^./, str => str.toUpperCase())
  }, [locale])
  
  return {
    t,
    locale,
    setLocale,
    isZhHK: locale === 'zh-HK',
  }
}

/**
 * 高階組件 - 為組件注入翻譯功能
 */
export function withTranslation<P extends object>(
  Component: React.ComponentType<P & ReturnType<typeof useTranslation>>
) {
  return function WithTranslationComponent(props: P) {
    const translation = useTranslation()
    return <Component {...props} {...translation} />
  }
}

export default useTranslation
