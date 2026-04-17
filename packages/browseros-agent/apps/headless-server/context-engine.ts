// Context Engine — อ่าน DOM อัตโนมัติ สรุปเนื้อหาหน้าเว็บ
// ตรวจจับ interactive elements ส่ง context ไปให้ AI

import { HeadlessBrowser } from './headless-browser'
import { SessionManager, type BrowserSession } from './session-manager'

/** ข้อมูลบริบทของหน้าเว็บ */
export interface PageContext {
  url: string
  title: string
  meta: {
    description?: string
    keywords?: string
    language?: string
  }
  headings: { level: number; text: string }[]
  links: { text: string; href: string }[]
  images: { alt: string; src: string }[]
  forms: { action: string; method: string; inputs: FormInput[] }[]
  interactiveElements: InteractiveElement[]
  textSummary: string
  timestamp: number
}

export interface FormInput {
  type: string
  name: string
  placeholder?: string
  required: boolean
  label?: string
}

export interface InteractiveElement {
  tag: string
  selector: string
  text: string
  type?: string
  role?: string
  ariaLabel?: string
  visible: boolean
}

/** ประวัติ context ของแต่ละ session */
export interface ContextHistoryEntry {
  timestamp: number
  url: string
  title: string
  summary: string
}

/**
 * Context Engine — อ่านและสรุปบริบทหน้าเว็บอัตโนมัติ
 * - อ่าน DOM เมื่อเปิดเว็บ / เปลี่ยนหน้า
 * - สรุปเนื้อหา (title, headings, forms, links, images)
 * - ตรวจจับ interactive elements
 * - เก็บ context history
 */
export class ContextEngine {
  private browser: HeadlessBrowser
  private sessions: SessionManager
  private currentContext = new Map<string, PageContext>()
  private contextHistory = new Map<string, ContextHistoryEntry[]>()
  private maxHistory = 50

  constructor(browser: HeadlessBrowser, sessions: SessionManager) {
    this.browser = browser
    this.sessions = sessions
  }

  /** อ่านบริบทหน้าเว็บปัจจุบัน */
  async extractContext(sessionId: string): Promise<PageContext> {
    const session = this.sessions.get(sessionId)
    if (!session) throw new Error(`ไม่พบ session ${sessionId}`)

    const [url, title, contextData] = await Promise.all([
      this.browser.getCurrentUrl(session.tabId),
      this.browser.getTitle(session.tabId),
      this.browser.evaluate(session.tabId, CONTEXT_EXTRACTION_JS),
    ])

    const ctx: PageContext = {
      url,
      title,
      meta: contextData?.meta || {},
      headings: contextData?.headings || [],
      links: (contextData?.links || []).slice(0, 50), // จำกัด 50 ลิงก์
      images: (contextData?.images || []).slice(0, 30),
      forms: contextData?.forms || [],
      interactiveElements: (contextData?.interactive || []).slice(0, 80),
      textSummary: (contextData?.textSummary || '').substring(0, 3000),
      timestamp: Date.now(),
    }

    // เก็บ context ปัจจุบัน
    this.currentContext.set(sessionId, ctx)

    // เพิ่มใน history
    const history = this.contextHistory.get(sessionId) || []
    history.push({
      timestamp: ctx.timestamp,
      url: ctx.url,
      title: ctx.title,
      summary: ctx.textSummary.substring(0, 500),
    })
    if (history.length > this.maxHistory) {
      history.splice(0, history.length - this.maxHistory)
    }
    this.contextHistory.set(sessionId, history)

    return ctx
  }

  /** ดึง context ปัจจุบัน (ไม่อ่านใหม่) */
  getCurrentContext(sessionId: string): PageContext | undefined {
    return this.currentContext.get(sessionId)
  }

  /** ดึงประวัติ context */
  getContextHistory(sessionId: string): ContextHistoryEntry[] {
    return this.contextHistory.get(sessionId) || []
  }

  /** สร้าง context summary สั้นสำหรับส่งให้ AI */
  buildAISummary(sessionId: string): string {
    const ctx = this.currentContext.get(sessionId)
    if (!ctx) return 'ยังไม่มีข้อมูลหน้าเว็บ'

    let parts: string[] = []
    parts.push(`หน้าเว็บ: ${ctx.title}`)
    parts.push(`URL: ${ctx.url}`)

    if (ctx.meta.description) {
      parts.push(`คำอธิบาย: ${ctx.meta.description}`)
    }

    if (ctx.headings.length > 0) {
      parts.push('หัวข้อ:')
      ctx.headings.slice(0, 10).forEach(h => {
        parts.push(`  H${h.level}: ${h.text}`)
      })
    }

    if (ctx.forms.length > 0) {
      parts.push(`ฟอร์ม: ${ctx.forms.length} ฟอร์ม`)
      ctx.forms.forEach((f, i) => {
        parts.push(`  ฟอร์ม ${i + 1}: ${f.inputs.length} ช่องกรอก`)
        f.inputs.forEach(inp => {
          parts.push(`    - ${inp.type} name="${inp.name}" ${inp.label ? `label="${inp.label}"` : ''} ${inp.placeholder ? `placeholder="${inp.placeholder}"` : ''}`)
        })
      })
    }

    if (ctx.interactiveElements.length > 0) {
      parts.push(`อิลิเมนต์ที่กดได้: ${ctx.interactiveElements.length} รายการ`)
      ctx.interactiveElements.slice(0, 20).forEach(el => {
        parts.push(`  <${el.tag}> ${el.text || el.ariaLabel || el.selector} ${el.type ? `[${el.type}]` : ''}`)
      })
    }

    if (ctx.links.length > 0) {
      parts.push(`ลิงก์: ${ctx.links.length} ลิงก์`)
      ctx.links.slice(0, 10).forEach(l => {
        parts.push(`  "${l.text}" → ${l.href}`)
      })
    }

    if (ctx.textSummary) {
      parts.push(`สรุปเนื้อหา:\n${ctx.textSummary.substring(0, 1500)}`)
    }

    return parts.join('\n')
  }

  /** ลบ context เมื่อปิด session */
  clearContext(sessionId: string): void {
    this.currentContext.delete(sessionId)
    this.contextHistory.delete(sessionId)
  }
}

// JavaScript ที่ inject ลงในหน้าเว็บเพื่อดึงบริบท
const CONTEXT_EXTRACTION_JS = `
(function() {
  const result = {
    meta: {},
    headings: [],
    links: [],
    images: [],
    forms: [],
    interactive: [],
    textSummary: ''
  };

  // Meta
  const desc = document.querySelector('meta[name="description"]');
  if (desc) result.meta.description = desc.content;
  const kw = document.querySelector('meta[name="keywords"]');
  if (kw) result.meta.keywords = kw.content;
  result.meta.language = document.documentElement.lang || '';

  // Headings
  document.querySelectorAll('h1,h2,h3,h4,h5,h6').forEach(h => {
    const text = h.innerText.trim();
    if (text) result.headings.push({ level: parseInt(h.tagName[1]), text: text.substring(0, 200) });
  });
  result.headings = result.headings.slice(0, 30);

  // Links
  document.querySelectorAll('a[href]').forEach(a => {
    const text = a.innerText.trim();
    if (text && text.length < 100) {
      result.links.push({ text, href: a.href });
    }
  });

  // Images
  document.querySelectorAll('img[src]').forEach(img => {
    result.images.push({ alt: img.alt || '', src: img.src });
  });

  // Forms
  document.querySelectorAll('form').forEach(form => {
    const inputs = [];
    form.querySelectorAll('input,select,textarea').forEach(el => {
      const label = el.id ? (document.querySelector('label[for="' + el.id + '"]')?.innerText?.trim() || '') : '';
      inputs.push({
        type: el.type || el.tagName.toLowerCase(),
        name: el.name || '',
        placeholder: el.placeholder || '',
        required: el.required || false,
        label
      });
    });
    result.forms.push({
      action: form.action || '',
      method: form.method || 'GET',
      inputs
    });
  });

  // Interactive elements
  const interactiveSelectors = 'button, [role="button"], input[type="submit"], input[type="button"], select, [onclick], [role="link"], [role="menuitem"], [role="tab"], a[href]';
  document.querySelectorAll(interactiveSelectors).forEach(el => {
    const text = (el.innerText || el.value || '').trim().substring(0, 100);
    let selector = el.tagName.toLowerCase();
    if (el.id) selector += '#' + el.id;
    else if (el.name) selector += '[name="' + el.name + '"]';
    else if (el.className && typeof el.className === 'string') {
      const cls = el.className.trim().split(/\\s+/).slice(0, 2).join('.');
      if (cls) selector += '.' + cls;
    }
    result.interactive.push({
      tag: el.tagName.toLowerCase(),
      selector,
      text,
      type: el.type || undefined,
      role: el.getAttribute('role') || undefined,
      ariaLabel: el.getAttribute('aria-label') || undefined,
      visible: !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length)
    });
  });

  // Text summary
  const body = document.body.cloneNode(true);
  body.querySelectorAll('script,style,noscript,svg,nav,footer,header').forEach(el => el.remove());
  result.textSummary = (body.innerText || '').replace(/\\s+/g, ' ').trim().substring(0, 3000);

  return result;
})()
`
