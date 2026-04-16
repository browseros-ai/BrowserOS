# 📋 แผนพัฒนาระบบ OAuth Login สำหรับ BrowserOS

> สร้างเมื่อ: 16 เม.ย. 2569
> สถานะ: ร่างแผน

---

## 1. สรุปสถานะปัจจุบัน

### 1.1 ระบบ OAuth ที่มีอยู่แล้วใน BrowserOS

BrowserOS **มีระบบ OAuth ที่สมบูรณ์อยู่แล้ว** สำหรับ provider ที่ใช้บริการแบบสมัครสมาชิก:

| Provider | Flow | สถานะ |
|----------|------|--------|
| ChatGPT Plus/Pro | PKCE (redirect) | ✅ ทำงานได้ |
| GitHub Copilot | Device Code | ✅ ทำงานได้ |
| Qwen Code | Device Code + PKCE | ✅ ทำงานได้ |

**สถาปัตยกรรมปัจจุบัน:**
- **Server-side** (`apps/server/src/lib/clients/oauth/`): Token Manager, Token Store (SQLite), Callback Server, Provider Config
- **Client-side** (`apps/agent/lib/llm-providers/`): OAuth Flow Hook, OAuth Status Hook, Client OAuth (Device Code + PKCE), Storage, Templates
- **API Routes** (`apps/server/src/api/routes/oauth.ts`): `GET /:provider/start`, `POST /:provider/token`, `GET /:provider/status`, `DELETE /:provider`

### 1.2 Provider ที่ใช้ API Key เท่านั้น (ยังไม่มี OAuth)

| Provider | วิธีล็อกอินปัจจุบัน | ต้องเพิ่ม OAuth? |
|----------|---------------------|-----------------|
| Z.AI GLM | API Key ธรรมดา | ⚠️ ต้องดูว่า Z.AI รองรับ OAuth ไหม |
| Anthropic | API Key ธรรมดา | ❌ ไม่มี OAuth สาธารณะ |
| OpenAI (Platform) | API Key ธรรมดา | ❌ ใช้ API Key อยู่ |
| Google Gemini | API Key ธรรมดา | ⚠️ Google มี OAuth แต่สำหรับ AI Studio ใช้ Key |
| OpenRouter | API Key ธรรมดา | ❌ ไม่มี OAuth |
| Ollama/LM Studio | ไม่ต้อง Auth | ❌ ไม่เกี่ยวข้อง |

### 1.3 สรุป OpenClaw Auth Pattern

OpenClaw ใช้ระบบ **Profile-based** ที่เก็บใน `openclaw.json`:
```json
"auth": {
  "profiles": {
    "zai:default": { "provider": "zai", "mode": "api_key" },
    "minimax:portal": { "provider": "minimax", "mode": "oauth" }
  }
}
```
- รองรับทั้ง `api_key` และ `oauth` mode
- แต่ละ provider มี profile แยกกัน (เช่น `zai:default`, `nvidia:default`)
- API keys เก็บใน `env` section
- มี `models.providers` สำหรับ config model-specific settings

---

## 2. สิ่งที่ต้องพัฒนาเพิ่มเติม

จากการศึกษา ระบบ OAuth ของ BrowserOS **ค่อนข้างสมบูรณ์แล้ว** สิ่งที่ต้องทำเพิ่มคือ:

### 2.1 เพิ่ม Provider ใหม่ที่รองรับ OAuth

#### 🟢 Z.AI (zai-glm)
Z.AI มี OAuth ผ่าน `open.bigmodel.cn` — ต้องเพิ่ม:

**ไฟล์ที่ต้องแก้ไข:**

| ไฟล์ | การเปลี่ยนแปลง |
|------|---------------|
| `apps/server/src/lib/clients/oauth/providers.ts` | เพิ่ม config สำหรับ `zai-glm` |
| `apps/agent/lib/llm-providers/providerTemplates.ts` | อัปเดต template ให้รองรับ OAuth mode |
| `apps/agent/lib/llm-providers/useOAuthProviderFlow.ts` | เพิ่ม flow config สำหรับ Z.AI |

**Z.AI OAuth Config (โครงสร้าง):**
```typescript
'zai-glm': {
  id: 'zai-glm',
  name: 'Z.AI GLM',
  clientId: '<ต้องขอจาก Z.AI>',
  authEndpoint: 'https://open.bigmodel.cn/oauth2/authorize',
  tokenEndpoint: 'https://open.bigmodel.cn/oauth2/token',
  scopes: ['openid', 'profile', 'email'],
  upstreamLLMProvider: 'zai-glm',
  authFlow: 'pkce',
}
```

#### 🟡 Google (Gemini)
Google มี OAuth 2.0 มาตรฐาน — แต่สำหรับการใช้ Gemini API โดยทั่วไปใช้ API Key ก็พอ จะเพิ่ม OAuth ก็ได้ถ้าต้องการให้ผู้ใช้ล็อกอินด้วย Google Account

#### 🔴 OpenAI (Platform) / Anthropic
- **OpenAI Platform**: ใช้ API Key เท่านั้น (ไม่มี OAuth สาธารณะสำหรับ Platform API)
- **Anthropic**: ใช้ API Key เท่านั้น (ไม่มี OAuth สาธารณะ)
- ไม่สามารถเพิ่ม OAuth ได้จนกว่าจะมี public OAuth endpoint

### 2.2 ปรับปรุงระบบ Session Management

ระบบปัจจุบันใช้ `OAuthTokenStore` (SQLite) เก็บ tokens ต่อ `browserosId` + `provider` ซึ่งเพียงพอแล้ว แต่อาจเพิ่ม:

- **Auto-refresh scheduler**: รีเฟรช token ก่อนหมดอายุอัตโนมัติ (ตอนนี้ refresh ตอนใช้งานเท่านั้น)
- **Token encryption**: เข้ารหัส tokens ใน SQLite (ตอนนี้เก็บแบบ plain text)
- **Multi-account support**: รองรับหลายบัญชีต่อ provider

### 2.3 UI/UX ปรับปรุง

- รวมหน้าตั้งค่า Provider: ทั้ง API Key และ OAuth อยู่ในที่เดียวกัน
- แสดงสถานะ OAuth บน provider card (เช่น "เชื่อมต่อแล้ว: user@email.com")
- ปุ่ม "เชื่อมต่อ" / "ตัดการเชื่อมต่อ" สำหรับ provider ที่รองรับ OAuth

---

## 3. ลำดับการพัฒนา

### Phase 1: เพิ่ม Z.AI OAuth (3-5 วัน)

| ขั้นตอน | เวลา | รายละเอียด |
|---------|------|-----------|
| 1.1 ขอ OAuth Client ID จาก Z.AI | 1 วัน | ติดต่อ Z.AI เพื่อขอ clientId + clientSecret |
| 1.2 เพิ่ม provider config ฝั่ง server | 0.5 วัน | แก้ `providers.ts` เพิ่ม `zai-glm` config |
| 1.3 ทดสอบ OAuth flow | 1 วัน | ทดสอบ PKCE flow กับ Z.AI endpoint |
| 1.4 อัปเดต client-side hooks | 0.5 วัน | เพิ่ม flow config ใน frontend |
| 1.5 อัปเดต UI | 1 วัน | ปุ่มเชื่อมต่อ Z.AI บน settings page |

### Phase 2: ปรับปรุง Session Management (2-3 วัน)

| ขั้นตอน | เวลา | รายละเอียด |
|---------|------|-----------|
| 2.1 เพิ่ม auto-refresh scheduler | 1 วัน | Cron-like refresh ก่อน token หมดอายุ |
| 2.2 เข้ารหัส tokens ใน SQLite | 1 วัน | ใช้ AES-256-GCM หรือ Bun crypto |
| 2.3 เพิ่ม multi-account support | 1 วัน | รองรับหลายบัญชีต่อ provider |

### Phase 3: UI/UX รวมศูนย์ (2-3 วัน)

| ขั้นตอน | เวลา | รายละเอียด |
|---------|------|-----------|
| 3.1 รวมหน้า Provider Settings | 1 วัน | ทั้ง API Key + OAuth ในหน้าเดียว |
| 3.2 แสดงสถานะ OAuth | 0.5 วัน | แสดง email + connection status |
| 3.3 Animation/Feedback | 0.5 วัน | Loading states, success/error feedback |

### Phase 4: Google OAuth (ถ้าต้องการ) (2-3 วัน)

| ขั้นตอน | เวลา | รายละเอียด |
|---------|------|-----------|
| 4.1 สร้าง GCP OAuth App | 0.5 วัน | ตั้งค่า Google Cloud Console |
| 4.2 เพิ่ม Google provider config | 0.5 วัน | PKCE flow กับ Google OAuth 2.0 |
| 4.3 ทดสอบ + UI | 1 วัน | ทดสอบ flow + ปุ่มเชื่อมต่อ |

---

## 4. สถาปัตยกรรมที่แนะนำ

```
┌─────────────────────────────────────────┐
│           BrowserOS Extension           │
│  (Frontend - React Hooks)               │
│                                         │
│  useOAuthProviderFlow ──→ UI Components │
│  useOAuthStatus ──→ Status Display      │
│  useLlmProviders ──→ Provider CRUD      │
│  client-oauth ──→ Device Code + PKCE    │
└──────────────┬──────────────────────────┘
               │ HTTP
               ▼
┌─────────────────────────────────────────┐
│           BrowserOS Server              │
│  (Backend - Hono + Bun)                 │
│                                         │
│  OAuth Routes (/oauth/:provider/*)      │
│  OAuthTokenManager (flow + refresh)     │
│  OAuthTokenStore (SQLite persistence)   │
│  OAuthCallbackServer (PKCE redirect)    │
└──────────────┬──────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────┐
│         OAuth Provider Endpoints        │
│                                         │
│  OpenAI Auth (PKCE)                     │
│  GitHub Device Code                     │
│  Qwen Device Code + PKCE                │
│  Z.AI OAuth (PKCE) ← เพิ่มใหม่          │
│  Google OAuth 2.0 (PKCE) ← เพิ่มใหม่    │
└─────────────────────────────────────────┘
```

---

## 5. ไฟล์สำคัญทั้งหมด

### ไฟล์ Server-side (Backend)
| ไฟล์ | หน้าที่ |
|------|--------|
| `apps/server/src/lib/clients/oauth/providers.ts` | **กำหนดค่า OAuth providers** ← เพิ่ม Z.AI, Google ที่นี่ |
| `apps/server/src/lib/clients/oauth/token-manager.ts` | จัดการ OAuth flow (PKCE + Device Code) |
| `apps/server/src/lib/clients/oauth/token-store.ts` | เก็บ tokens ใน SQLite |
| `apps/server/src/lib/clients/oauth/callback-server.ts` | รับ PKCE callback |
| `apps/server/src/lib/clients/oauth/index.ts` | Initialize OAuth system |
| `apps/server/src/api/routes/oauth.ts` | API endpoints สำหรับ OAuth |

### ไฟล์ Client-side (Frontend)
| ไฟล์ | หน้าที่ |
|------|--------|
| `apps/agent/lib/llm-providers/useOAuthProviderFlow.ts` | Hook สำหรับ OAuth flow |
| `apps/agent/lib/llm-providers/useOAuthStatus.ts` | Hook สำหรับตรวจสอบสถานะ |
| `apps/agent/lib/llm-providers/client-oauth.ts` | Client-side Device Code + PKCE |
| `apps/agent/lib/llm-providers/providerTemplates.ts` | Template สำหรับแต่ละ provider |
| `apps/agent/lib/llm-providers/useLlmProviders.ts` | Hook สำหรับจัดการ providers |
| `apps/agent/lib/llm-providers/types.ts` | Type definitions |
| `apps/agent/lib/llm-providers/storage.ts` | Storage (WXT) |

---

## 6. การประมาณเวลาทั้งหมด

| Phase | เวลา | หมายเหตุ |
|-------|------|----------|
| Phase 1: Z.AI OAuth | 3-5 วัน | ขึ้นกับการได้ Client ID |
| Phase 2: Session ปรับปรุง | 2-3 วัน | Optional |
| Phase 3: UI รวมศูนย์ | 2-3 วัน | Optional |
| Phase 4: Google OAuth | 2-3 วัน | Optional |
| **รวม (Phase 1 เท่านั้น)** | **3-5 วัน** | **ขั้นต่ำที่แนะนำ** |
| **รวม (ทั้งหมด)** | **9-14 วัน** | **แบบครบถ้วน** |

---

## 7. ข้อสังเกตสำคัญ

1. **ระบบ OAuth มีอยู่แล้ว** — ไม่ต้องสร้างใหม่ทั้งหมด แค่เพิ่ม provider
2. **OpenAI/Anthropic ไม่มี OAuth สาธารณะ** — ต้องใช้ API Key ต่อไป
3. **สถาปัตยกรรมดีแล้ว** — แยก server/client ชัดเจน, รองรับทั้ง PKCE และ Device Code
4. **ควรเริ่มจาก Z.AI** เพราะเป็น provider หลักที่ใช้อยู่
5. **Google OAuth เพิ่มได้ง่าย** เพราะใช้ PKCE flow เดียวกับ ChatGPT
