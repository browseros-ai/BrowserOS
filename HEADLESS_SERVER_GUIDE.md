# 🖥️ BrowserOS Headless Server Mode — คู่มือติดตั้งและใช้งาน

## คืออะไร?

Headless Server Mode ช่วยให้ BrowserOS รันบน Linux server ได้โดยไม่ต้องมีหน้าจอ (GUI) ใช้ CDP (Chrome DevTools Protocol) ควบคุม Chromium เบื้องหลัง รับคำสั่งผ่าน HTTP API และ WebSocket

## ข้อกำหนด

- **ระบบปฏิบัติการ:** Linux (Ubuntu/Debian แนะนำ)
- **Runtime:** Bun 1.3+
- **Chromium:** ต้องติดตั้งไว้ (เช็คด้วย `which chromium-browser`)

## ติดตั้ง Chromium

```bash
# Ubuntu/Debian
sudo apt update && sudo apt install -y chromium-browser

# หรือใช้ snap
sudo snap install chromium
```

## วิธีเริ่ม Server

### 1. ตั้งค่า environment

```bash
cd /home/admin/BrowserOS
cp .env.server.example .env.server
# แก้ไข .env.server ตามต้องการ
nano .env.server
```

### 2. เริ่ม server

```bash
# วิธีที่ 1: ใช้ npm script (แนะนำ)
cd packages/browseros-agent/apps/agent
bun run server

# วิธีที่ 2: เริ่มโดยตรง
cd /home/admin/BrowserOS
bun --env-file=.env.server packages/browseros-agent/apps/headless-server/index.ts

# โหมด debug (แสดง log ละเอียด)
bun run server:dev
```

### 3. เริ่มเป็น background service

```bash
# ใช้ nohup
nohup bun run server > browseros-headless.log 2>&1 &

# หรือใช้ systemd (แนะนำสำหรับ production)
```

## API Endpoints

### สถานะ Server

```bash
# ตรวจสอบว่า server ทำงานอยู่
curl http://localhost:3100/api/status
```

### สร้าง Session

```bash
curl -X POST http://localhost:3100/api/session \
  -H "Content-Type: application/json" \
  -d '{"startUrl": "https://example.com"}'
# ได้ sessionId กลับมา
```

### เปิดเว็บ

```bash
curl -X POST http://localhost:3100/api/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://google.com", "sessionId": "sess_xxx"}'
```

### คลิก Element

```bash
curl -X POST http://localhost:3100/api/click \
  -H "Content-Type: application/json" \
  -d '{"sessionId": "sess_xxx", "selector": "#search-button"}'
```

### พิมพ์ข้อความ

```bash
curl -X POST http://localhost:3100/api/type \
  -H "Content-Type: application/json" \
  -d '{"sessionId": "sess_xxx", "selector": "input[name=q]", "text": "BrowserOS"}'
```

### จับภาพหน้าจอ

```bash
# ได้ base64 image กลับ
curl -X POST http://localhost:3100/api/screenshot \
  -H "Content-Type: application/json" \
  -d '{"sessionId": "sess_xxx"}'

# ได้ไฟล์ PNG โดยตรง
curl -X POST http://localhost:3100/api/screenshot \
  -H "Content-Type: application/json" \
  -d '{"sessionId": "sess_xxx", "format": "binary"}' \
  --output screenshot.png
```

### ดึงข้อมูลจากหน้าเว็บ

```bash
# ดึงข้อความทั้งหน้า
curl -X POST http://localhost:3100/api/extract \
  -H "Content-Type: application/json" \
  -d '{"sessionId": "sess_xxx"}'

# ดึงข้อความจากบางส่วน
curl -X POST http://localhost:3100/api/extract \
  -H "Content-Type: application/json" \
  -d '{"sessionId": "sess_xxx", "selector": ".article-content"}'

# ดึง HTML
curl -X POST http://localhost:3100/api/extract \
  -H "Content-Type: application/json" \
  -d '{"sessionId": "sess_xxx", "selector": "main", "type": "html"}'

# รัน JavaScript
curl -X POST http://localhost:3100/api/extract \
  -H "Content-Type: application/json" \
  -d '{"sessionId": "sess_xxx", "expression": "document.querySelectorAll(\"a\").length"}'
```

### กรอกฟอร์ม

```bash
curl -X POST http://localhost:3100/api/fill-form \
  -H "Content-Type: application/json" \
  -d '{
    "sessionId": "sess_xxx",
    "fields": {
      "input[name=username]": "admin",
      "input[name=password]": "secret123"
    },
    "submitSelector": "button[type=submit]"
  }'
```

### รัน JavaScript

```bash
curl -X POST http://localhost:3100/api/evaluate \
  -H "Content-Type: application/json" \
  -d '{"sessionId": "sess_xxx", "expression": "window.location.href"}'
```

### ปิด Session

```bash
curl -X DELETE http://localhost:3100/api/session \
  -H "Content-Type: application/json" \
  -d '{"sessionId": "sess_xxx"}'
```

## WebSocket

เชื่อมต่อไปที่ `ws://localhost:3100/ws` แล้วส่ง JSON:

```json
{
  "id": "1",
  "action": "navigate",
  "data": {
    "sessionId": "sess_xxx",
    "url": "https://example.com"
  }
}
```

### คำสั่งที่รองรับผ่าน WebSocket

| คำสั่ง | ข้อมูลที่ต้องส่ง |
|--------|------------------|
| `session.create` | `{ sessionId?, startUrl? }` |
| `session.destroy` | `{ sessionId }` |
| `navigate` | `{ sessionId, url, wait? }` |
| `click` | `{ sessionId, selector }` |
| `type` | `{ sessionId, selector, text }` |
| `screenshot` | `{ sessionId, quality?, fullPage? }` |
| `extract` | `{ sessionId, selector?, type?, expression? }` |
| `fillForm` | `{ sessionId, fields, submitSelector? }` |
| `evaluate` | `{ sessionId, expression }` |
| `status` | `{}` |

## การยืนยันตัวตน

ถ้าตั้ง `HEADLESS_API_KEY` ใน `.env.server` จะต้องส่ง header:

```
Authorization: Bearer your_api_key_here
```

## ตั้งค่า Environment

| ตัวแปร | ค่าเริ่มต้น | คำอธิบาย |
|---------|------------|-----------|
| `HEADLESS_PORT` | `3100` | พอร์ต server |
| `HEADLESS_HOST` | `0.0.0.0` | โฮสต์ที่ bind |
| `CHROMIUM_PATH` | auto-detect | พาธ Chromium |
| `HEADLESS_DEBUG` | `false` | โหมด debug |
| `HEADLESS_API_KEY` | (ว่าง) | API key |
| `PAGE_LOAD_TIMEOUT` | `30000` | รอโหลดหน้า (ms) |
| `WAIT_TIMEOUT` | `10000` | รอ element (ms) |
| `VIEWPORT_WIDTH` | `1280` | ความกว้าง |
| `VIEWPORT_HEIGHT` | `720` | ความสูง |
| `USER_DATA_DIR` | `/tmp/browseros-headless-user-data` | เก็บ cookies/state |

## โครงสร้างไฟล์

```
packages/browseros-agent/apps/headless-server/
├── index.ts              ← entry point (เริ่ม server)
├── config.ts             ← ตั้งค่า server
├── headless-browser.ts   ← ควบคุม Chromium ผ่าน CDP
├── session-manager.ts    ← จัดการ sessions
├── api-routes.ts         ← HTTP API endpoints
└── websocket-handler.ts  ← WebSocket handler
```

## ข้อจำกัด

- ไม่รองรับ BrowserOS AI Agent (เฉพาะ browser control)
- ไม่มี extension APIs (`chrome.browserOS.*`)
- ใช้ CDP โดยตรงแทน BrowserOSAdapter
- ต้องติดตั้ง Chromium แยก

## ตัวอย่าง: สคริปต์ค้นหา Google

```bash
# 1. สร้าง session และเปิด Google
RESPONSE=$(curl -s -X POST http://localhost:3100/api/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://google.com"}')
SESSION_ID=$(echo $RESPONSE | jq -r '.sessionId')

# 2. พิมพ์คำค้นหา
curl -s -X POST http://localhost:3100/api/type \
  -H "Content-Type: application/json" \
  -d "{\"sessionId\": \"$SESSION_ID\", \"selector\": \"textarea[name=q]\", \"text\": \"BrowserOS\"}"

# 3. กดค้นหา
curl -s -X POST http://localhost:3100/api/click \
  -H "Content-Type: application/json" \
  -d "{\"sessionId\": \"$SESSION_ID\", \"selector\": \"input[name=btnK]\"}"

# 4. รอแล้วจับภาพ
sleep 2
curl -s -X POST http://localhost:3100/api/screenshot \
  -H "Content-Type: application/json" \
  -d "{\"sessionId\": \"$SESSION_ID\", \"format\": \"binary\"}" \
  --output google-results.png

# 5. ดึงข้อความผลลัพธ์
curl -s -X POST http://localhost:3100/api/extract \
  -H "Content-Type: application/json" \
  -d "{\"sessionId\": \"$SESSION_ID\", \"selector\": \"#search\"}"

# 6. ปิด session
curl -s -X DELETE http://localhost:3100/api/session \
  -H "Content-Type: application/json" \
  -d "{\"sessionId\": \"$SESSION_ID\"}"
```
