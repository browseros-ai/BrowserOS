# 📖 คู่มือเชื่อม OpenClaw + BrowserOS ผ่าน MCP

## MCP คืออะไร?

MCP (Model Context Protocol) คือมาตรฐานที่ให้ AI agent เรียกใช้เครื่องมือภายนอกได้ เหมือนเสียบปลั๊กอุปกรณ์เข้ากับ AI

## สถาปัตยกรรม

```
OpenClaw (AI) ←→ MCP Server ←→ Headless Server ←→ Chromium
```

1. **OpenClaw** ส่งคำสั่งผ่าน MCP protocol
2. **MCP Server** แปลงเป็น HTTP API call
3. **Headless Server** ควบคุม Chromium จริง
4. **Chromium** เปิดเว็บ คลิก พิมพ์ จับภาพ

## วิธีติดตั้ง

### 1. เริ่ม Headless Server ก่อน

```bash
cd /home/admin/BrowserOS
bun run packages/browseros-agent/apps/headless-server/index.ts
```

เซิร์ฟเวอร์จะรันที่ `http://127.0.0.1:3100`

### 2. เลือก Transport

#### แบบ A: stdio (แนะนำสำหรับใช้บนเครื่องเดียวกัน)

MCP server รันเป็น child process ของ OpenClaw — สื่อสารผ่าน stdin/stdout

#### แบบ B: SSE (สำหรับเครื่องคนละเครื่อง)

MCP server รันเป็น HTTP server ต่างหาก — OpenClaw เชื่อมผ่าน HTTP

### 3. ตั้งค่า OpenClaw

เพิ่มในไฟล์ `.openclaw/openclaw.json`:

#### stdio mode:

```json
{
  "mcp": {
    "servers": {
      "browseros": {
        "transport": "stdio",
        "command": "bun",
        "args": ["run", "/home/admin/BrowserOS/packages/browseros-agent/apps/headless-server/mcp-server.ts"],
        "env": {
          "HEADLESS_SERVER_URL": "http://127.0.0.1:3100"
        }
      }
    }
  }
}
```

#### SSE mode:

```bash
# เริ่ม MCP server แบบ SSE ที่พอร์ต 3200
MCP_TRANSPORT=sse MCP_PORT=3200 bun run packages/browseros-agent/apps/headless-server/mcp-server.ts
```

```json
{
  "mcp": {
    "servers": {
      "browseros": {
        "transport": "sse",
        "url": "http://127.0.0.1:3200/sse"
      }
    }
  }
}
```

## Tools ทั้ง 14 ตัว

| Tool | หน้าที่ |
|------|---------|
| `browser_navigate` | เปิดเว็บไซต์ |
| `browser_click` | คลิก element |
| `browser_type` | พิมพ์ข้อความ |
| `browser_screenshot` | จับภาพหน้าจอ |
| `browser_extract` | ดึงข้อมูลจากหน้าเว็บ |
| `browser_fill_form` | กรอกฟอร์ม |
| `browser_get_text` | อ่านข้อความบนหน้าเว็บ |
| `browser_wait` | รอ element ปรากฏ |
| `browser_execute` | รัน JavaScript |
| `browser_get_cookies` | ดึง cookies |
| `browser_set_cookies` | ตั้ง cookies |
| `browser_tab_list` | ดูแท็บทั้งหมด |
| `browser_tab_switch` | สลับแท็บ |
| `browser_tab_close` | ปิดแท็บ |

## ตัวอย่างการใช้

### เปิดเว็บและดึงข้อมูล
```
AI: ขอเปิด google.com แล้วดึงข้อความบนหน้าเว็บ

→ browser_navigate({ url: "https://www.google.com" })
→ browser_get_text({})
```

### กรอกฟอร์ม
```
AI: กรอกฟอร์มล็อกอินให้  user=admin pass=1234

→ browser_fill_form({
    fields: {
      "#username": "admin",
      "#password": "1234"
    },
    submitSelector: "#login-button"
  })
```

### จับภาพหน้าจอ
```
AI: จับภาพหน้าจอให้หน่อย

→ browser_screenshot({ fullPage: true })
```

## การจัดการ Session

- MCP server สร้าง browser session อัตโนมัติเมื่อเรียก tool ครั้งแรก
- แต่ละ MCP client มี session ของตัวเอง (แยกกัน)
- Session ถูกเก็บไว้ตลอดการเชื่อมต่อ ไม่ต้องสร้างใหม่ทุกครั้ง
- ใช้ `browser_tab_list` เพื่อดู session ทั้งหมด

## ความปลอดภัย

- ตั้ง `HEADLESS_API_KEY` เพื่อป้องกันการเข้าถึง headless server โดยไม่ได้รับอนุญาต
- MCP server ผ่าน stdio จะรันบนเครื่องเดียวกัน — ไม่เปิดพอร์ตออกนอกเครื่อง
- SSE mode ควรใช้ผ่าน VPN หรือ private network เท่านั้น

## แก้ปัญหา

### Headless server ไม่ตอบ
```bash
curl http://127.0.0.1:3100/api/status
```

### MCP server ไม่เริ่ม
```bash
bun run packages/browseros-agent/apps/headless-server/mcp-server.ts
```
ดู error message ที่ stderr

### Chromium ไม่เริ่ม
ตรวจสอบว่าติดตั้ง Chromium แล้ว:
```bash
which chromium-browser
```
