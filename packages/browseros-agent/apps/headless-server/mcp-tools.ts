/**
 * MCP Tool Definitions สำหรับ BrowserOS
 *
 * นิยามทุก tool ที่ MCP server เปิดเผยให้ AI agent เรียกใช้
 * แต่ละ tool ประกอบด้วย:
 *   - name: ชื่อ tool
 *   - description: คำอธิบาย (ภาษาไทย)
 *   - inputSchema: JSON Schema ของ parameter
 *   - handler: ฟังก์ชันที่เรียก headless server API
 */

// ─── Types ────────────────────────────────────────────────

/** ฟังก์ชันเรียก headless server API */
export type CallAPI = (path: string, body?: Record<string, unknown>) => Promise<any>

/** Context ที่ส่งให้ handler */
export interface ToolContext {
  sessionId?: string
  callAPI: CallAPI
  setSessionId: (id: string) => Promise<void>
}

/** ฟังก์ชันจัดการ tool request */
export type McpToolHandler = (
  args: Record<string, unknown>,
  ctx: ToolContext,
) => Promise<any>

/** นิยาม MCP tool */
export interface McpToolDefinition {
  name: string
  description: string
  inputSchema: Record<string, unknown>
  handler: McpToolHandler
}

// ─── Helper ───────────────────────────────────────────────

/** ตรวจสอบว่ามี session แล้ว ถ้ายังไม่มีให้สร้างใหม่อัตโนมัติ */
async function ensureSession(ctx: ToolContext): Promise<string> {
  if (ctx.sessionId) return ctx.sessionId
  const result = await ctx.callAPI('/api/session', {})
  await ctx.setSessionId(result.sessionId)
  return result.sessionId
}

// ─── Tool definitions ─────────────────────────────────────

const browser_navigate: McpToolDefinition = {
  name: 'browser_navigate',
  description: 'เปิดเว็บไซต์ — นำทาง browser ไปยัง URL ที่ระบุ',
  inputSchema: {
    type: 'object',
    properties: {
      url: { type: 'string', description: 'URL ที่ต้องการเปิด' },
      wait: { type: 'boolean', description: 'รอให้หน้าโหลดเสร็จ (default: true)', default: true },
    },
    required: ['url'],
  },
  handler: async (args, ctx) => {
    const sessionId = await ensureSession(ctx)
    return ctx.callAPI('/api/navigate', { url: args.url, sessionId, wait: args.wait ?? true })
  },
}

const browser_click: McpToolDefinition = {
  name: 'browser_click',
  description: 'คลิก element บนหน้าเว็บ — ใช้ CSS selector ระบุ element ที่ต้องการคลิก',
  inputSchema: {
    type: 'object',
    properties: {
      selector: { type: 'string', description: 'CSS selector ของ element ที่จะคลิก' },
    },
    required: ['selector'],
  },
  handler: async (args, ctx) => {
    const sessionId = await ensureSession(ctx)
    return ctx.callAPI('/api/click', { sessionId, selector: args.selector })
  },
}

const browser_type: McpToolDefinition = {
  name: 'browser_type',
  description: 'พิมพ์ข้อความลงใน input field — เลือก element ด้วย CSS selector แล้วพิมพ์ข้อความ',
  inputSchema: {
    type: 'object',
    properties: {
      selector: { type: 'string', description: 'CSS selector ของ input element' },
      text: { type: 'string', description: 'ข้อความที่จะพิมพ์' },
    },
    required: ['selector', 'text'],
  },
  handler: async (args, ctx) => {
    const sessionId = await ensureSession(ctx)
    return ctx.callAPI('/api/type', { sessionId, selector: args.selector, text: args.text })
  },
}

const browser_screenshot: McpToolDefinition = {
  name: 'browser_screenshot',
  description: 'จับภาพหน้าจอปัจจุบัน — คืนรูปเป็น base64',
  inputSchema: {
    type: 'object',
    properties: {
      fullPage: { type: 'boolean', description: 'จับทั้งหน้าหรือเฉพาะส่วนที่มองเห็น (default: false)' },
    },
  },
  handler: async (args, ctx) => {
    const sessionId = await ensureSession(ctx)
    return ctx.callAPI('/api/screenshot', { sessionId, format: 'base64', fullPage: args.fullPage ?? false })
  },
}

const browser_extract: McpToolDefinition = {
  name: 'browser_extract',
  description: 'ดึงข้อมูลจากหน้าเว็บ — เลือกดึง text, HTML หรือผลจาก JavaScript expression',
  inputSchema: {
    type: 'object',
    properties: {
      selector: { type: 'string', description: 'CSS selector (ถ้าไม่ระบุจะดึงจาก body)' },
      type: { type: 'string', enum: ['text', 'html', 'json'], description: 'ประเภทข้อมูลที่ต้องการ (default: text)' },
      expression: { type: 'string', description: 'JavaScript expression สำหรับดึงข้อมูลแบบ custom' },
    },
  },
  handler: async (args, ctx) => {
    const sessionId = await ensureSession(ctx)
    return ctx.callAPI('/api/extract', {
      sessionId,
      selector: args.selector,
      type: args.type || 'text',
      expression: args.expression,
    })
  },
}

const browser_fill_form: McpToolDefinition = {
  name: 'browser_fill_form',
  description: 'กรอกฟอร์ม — ใส่ข้อมูลหลายฟิลด์พร้อมกัน และสามารถกด submit ได้',
  inputSchema: {
    type: 'object',
    properties: {
      fields: {
        type: 'object',
        description: 'Object ที่ key คือ CSS selector และ value คือข้อมูลที่จะกรอก',
        additionalProperties: { type: 'string' },
      },
      submitSelector: { type: 'string', description: 'CSS selector ของปุ่ม submit (ถ้าต้องการกด)' },
    },
    required: ['fields'],
  },
  handler: async (args, ctx) => {
    const sessionId = await ensureSession(ctx)
    return ctx.callAPI('/api/fill-form', {
      sessionId,
      fields: args.fields,
      submitSelector: args.submitSelector,
    })
  },
}

const browser_get_text: McpToolDefinition = {
  name: 'browser_get_text',
  description: 'อ่านข้อความบนหน้าเว็บ — ดึงเฉพาะ text content จาก element ที่ระบุ หรือทั้งหน้า',
  inputSchema: {
    type: 'object',
    properties: {
      selector: { type: 'string', description: 'CSS selector (ถ้าไม่ระบุจะดึงทั้งหน้า)' },
    },
  },
  handler: async (args, ctx) => {
    const sessionId = await ensureSession(ctx)
    return ctx.callAPI('/api/extract', { sessionId, selector: args.selector, type: 'text' })
  },
}

const browser_wait: McpToolDefinition = {
  name: 'browser_wait',
  description: 'รอให้ element ปรากฏบนหน้าเว็บ — ใช้รอให้หน้าโหลดเสร็จหรือรอ element แสดงผล',
  inputSchema: {
    type: 'object',
    properties: {
      selector: { type: 'string', description: 'CSS selector ของ element ที่รอ' },
      timeout: { type: 'number', description: 'เวลารอสูงสุดเป็นมิลลิวินาที (default: 10000)' },
    },
    required: ['selector'],
  },
  handler: async (args, ctx) => {
    const sessionId = await ensureSession(ctx)
    const timeout = args.timeout ?? 10000
    const selector = args.selector as string

    // ใช้ evaluate เพื่อรอ element
    const result = await ctx.callAPI('/api/evaluate', {
      sessionId,
      expression: `new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('รอ element เกินเวลา')), ${timeout});
        const check = () => {
          if (document.querySelector('${selector}')) {
            clearTimeout(timer);
            resolve(true);
          } else {
            requestAnimationFrame(check);
          }
        };
        check();
      })`,
    })
    return { success: true, found: true, selector }
  },
}

const browser_execute: McpToolDefinition = {
  name: 'browser_execute',
  description: 'รัน JavaScript บนหน้าเว็บ — สำหรับดึงข้อมูลหรือจัดการ DOM แบบ custom',
  inputSchema: {
    type: 'object',
    properties: {
      expression: { type: 'string', description: 'JavaScript expression ที่ต้องการรัน' },
    },
    required: ['expression'],
  },
  handler: async (args, ctx) => {
    const sessionId = await ensureSession(ctx)
    return ctx.callAPI('/api/evaluate', { sessionId, expression: args.expression })
  },
}

const browser_get_cookies: McpToolDefinition = {
  name: 'browser_get_cookies',
  description: 'ดึง cookies ทั้งหมดของหน้าเว็บปัจจุบัน',
  inputSchema: {
    type: 'object',
    properties: {},
  },
  handler: async (args, ctx) => {
    const sessionId = await ensureSession(ctx)
    // GET /api/cookies/:sessionId
    return ctx.callAPI(`/api/cookies/${sessionId}`)
  },
}

const browser_set_cookies: McpToolDefinition = {
  name: 'browser_set_cookies',
  description: 'ตั้งค่า cookies — เพิ่มหรือแก้ไข cookies ของหน้าเว็บ',
  inputSchema: {
    type: 'object',
    properties: {
      cookies: {
        type: 'array',
        description: 'รายการ cookies ที่จะตั้ง',
        items: {
          type: 'object',
          properties: {
            name: { type: 'string' },
            value: { type: 'string' },
            domain: { type: 'string' },
            path: { type: 'string' },
          },
          required: ['name', 'value'],
        },
      },
    },
    required: ['cookies'],
  },
  handler: async (args, ctx) => {
    const sessionId = await ensureSession(ctx)
    return ctx.callAPI('/api/cookies', { sessionId, cookies: args.cookies })
  },
}

const browser_tab_list: McpToolDefinition = {
  name: 'browser_tab_list',
  description: 'ดูรายการแท็บที่เปิดอยู่ทั้งหมด',
  inputSchema: {
    type: 'object',
    properties: {},
  },
  handler: async (args, ctx) => {
    return ctx.callAPI('/api/status')
  },
}

const browser_tab_switch: McpToolDefinition = {
  name: 'browser_tab_switch',
  description: 'สลับไปยังแท็บอื่น — เปลี่ยน session ไปยัง tab ที่ระบุ',
  inputSchema: {
    type: 'object',
    properties: {
      sessionId: { type: 'string', description: 'ID ของ session/แท็บที่ต้องการสลับไป' },
    },
    required: ['sessionId'],
  },
  handler: async (args, ctx) => {
    await ctx.setSessionId(args.sessionId as string)
    return { success: true, activeSessionId: args.sessionId }
  },
}

const browser_tab_close: McpToolDefinition = {
  name: 'browser_tab_close',
  description: 'ปิดแท็บ — ปิด session/แท็บที่ระบุ',
  inputSchema: {
    type: 'object',
    properties: {
      sessionId: { type: 'string', description: 'ID ของ session ที่ต้องการปิด' },
    },
  },
  handler: async (args, ctx) => {
    const sid = (args.sessionId as string) || ctx.sessionId
    if (!sid) return { success: false, error: 'ไม่มี session ที่จะปิด' }
    // DELETE /api/session
    const result = await ctx.callAPI('/api/session', { sessionId: sid })
    // ถ้าปิด session ปัจจุบัน ให้เคลียร์
    if (ctx.sessionId === sid) {
      // clientSessions จะถูกเคลียร์ผ่าน context
    }
    return result
  },
}

// ─── Export ───────────────────────────────────────────────

export const ALL_MCP_TOOLS: McpToolDefinition[] = [
  browser_navigate,
  browser_click,
  browser_type,
  browser_screenshot,
  browser_extract,
  browser_fill_form,
  browser_get_text,
  browser_wait,
  browser_execute,
  browser_get_cookies,
  browser_set_cookies,
  browser_tab_list,
  browser_tab_switch,
  browser_tab_close,
]
