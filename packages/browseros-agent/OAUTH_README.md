# 🔐 ระบบ OAuth/Auth - BrowserOS

## ภาพรวม

ระบบจัดการการยืนยันตัวตนสำหรับ BrowserOS รองรับทั้ง **API Key** และ **OAuth** ในระบบเดียวกัน

### Provider ที่รองรับ

| Provider | วิธีล็อกอิน | สถานะ |
|----------|-------------|-------|
| Google (Gemini) | OAuth PKCE | ✅ พร้อมใช้ |
| ChatGPT Plus/Pro | OAuth PKCE | ✅ มีอยู่แล้ว |
| GitHub Copilot | OAuth Device Code | ✅ มีอยู่แล้ว |
| Qwen Code | OAuth Device Code | ✅ มีอยู่แล้ว |
| Anthropic | API Key | ✅ พร้อมใช้ |
| OpenAI | API Key | ✅ พร้อมใช้ |
| OpenRouter | API Key | ✅ พร้อมใช้ |
| Ollama | ไม่ต้องล็อกอิน | ✅ พร้อมใช้ |

---

## สถาปัตยกรรม

```
┌─────────────────────────────────────────┐
│           BrowserOS Extension           │
│  ┌──────────────┐  ┌─────────────────┐  │
│  │ useOAuth     │  │ useOAuth        │  │
│  │ ProviderFlow │  │ Status          │  │
│  └──────┬───────┘  └────────┬────────┘  │
│         │                   │            │
│  ┌──────▼───────────────────▼────────┐  │
│  │     client-oauth.ts              │  │
│  │  (Device Code + PKCE client)     │  │
│  └──────────────┬───────────────────┘  │
└─────────────────┼───────────────────────┘
                  │ HTTP
┌─────────────────▼───────────────────────┐
│            BrowserOS Server             │
│  ┌──────────────────────────────────┐   │
│  │     OAuth Routes (/oauth/*)      │   │
│  └──────────────┬───────────────────┘   │
│  ┌──────────────▼───────────────────┐   │
│  │     OAuthTokenManager            │   │
│  │  - PKCE flow                     │   │
│  │  - Device Code flow              │   │
│  │  - Auto token refresh            │   │
│  └──────────────┬───────────────────┘   │
│  ┌──────────────▼───────────────────┐   │
│  │     AuthProfileManager           │   │
│  │  - API Key storage               │   │
│  │  - OAuth profile tracking        │   │
│  └──────────────┬───────────────────┘   │
│  ┌──────────────▼───────────────────┐   │
│  │     OAuthTokenStore (SQLite)     │   │
│  └──────────────────────────────────┘   │
└─────────────────────────────────────────┘
```

---

## การใช้งาน

### 1. ล็อกอินด้วย Google

```typescript
// ฝั่ง extension - ใช้ hook
import { useOAuthProviderFlow } from '@/lib/llm-providers/useOAuthProviderFlow'
import { googleOAuthFlowConfig } from '@/lib/llm-providers/google-oauth-config'

const { status, startOAuthFlow, disconnect } = useOAuthProviderFlow(
  googleOAuthFlowConfig,
  providers,
  saveProvider,
)

// เริ่มล็อกอิน
startOAuthFlow(agentServerUrl)

// ตรวจสอบสถานะ
console.log(status?.authenticated) // true/false
console.log(status?.email) // user@gmail.com

// ล็อกเอาท์
disconnect()
```

### 2. ใช้ API Key แทน OAuth

```typescript
// ผ่าน API
await fetch('http://localhost:3000/auth-profiles/google/api-key', {
  method: 'POST',
  body: JSON.stringify({ apiKey: 'AIza...' }),
})
```

### 3. ดึง credentials ใน provider

```typescript
// ฝั่ง server - AuthProfileManager จะตรวจสอบ OAuth ก่อน, ถ้าหมดอายุจะลอง API Key
const credentials = await profileManager.getCredentials('google')
if (credentials?.type === 'oauth') {
  // ใช้ credentials.accessToken
} else if (credentials?.type === 'api-key') {
  // ใช้ credentials.apiKey
}
```

---

## ไฟล์สำคัญ

### ฝั่ง Server (`apps/server/src/`)
- `lib/clients/oauth/providers.ts` — กำหนดค่า OAuth ของแต่ละ provider
- `lib/clients/oauth/token-manager.ts` — จัดการ token lifecycle (PKCE, Device Code, refresh)
- `lib/clients/oauth/token-store.ts` — เก็บ token ใน SQLite
- `lib/clients/oauth/callback-server.ts` — callback server สำหรับ PKCE flow
- `lib/clients/oauth/auth-profiles.ts` — ระบบ Auth Profile รวม API Key + OAuth
- `lib/clients/oauth/google-userinfo.ts` — ดึงข้อมูลผู้ใช้ Google
- `api/routes/oauth.ts` — API endpoints สำหรับ OAuth
- `api/routes/auth-profiles.ts` — API endpoints สำหรับ Auth Profiles

### ฝั่ง Extension (`apps/agent/lib/`)
- `llm-providers/client-oauth.ts` — client-side device code flow
- `llm-providers/useOAuthProviderFlow.ts` — React hook สำหรับ OAuth flow
- `llm-providers/useOAuthStatus.ts` — React hook ตรวจสอบสถานะ OAuth
- `llm-providers/google-oauth-config.ts` — config สำหรับ Google OAuth flow
- `llm-providers/providerTemplates.ts` — provider templates
- `llm-providers/types.ts` — TypeScript types

---

## การเพิ่ม Provider ใหม่

### ถ้าใช้ OAuth (PKCE):
1. เพิ่ม config ใน `providers.ts` (OAUTH_PROVIDERS)
2. เพิ่ม URLs ใน `urls.ts` (EXTERNAL_URLS)
3. สร้าง flow config ใน `google-oauth-config.ts` เป็นต้นแบบ
4. เพิ่ม provider template ใน `providerTemplates.ts`

### ถ้าใช้ API Key:
1. เพิ่ม provider template ใน `providerTemplates.ts`
2. ผู้ใช้กรอก API Key ผ่าน settings
3. AuthProfileManager จะจัดการเอง

---

## Token Refresh อัตโนมัติ

ระบบตรวจสอบ token อัตโนมัติเมื่อเรียก `getCredentials()`:
1. ตรวจสอบว่า token หมดอายุหรือยัง
2. ถ้าหมดแล้ว → refresh อัตโนมัติด้วย refresh_token
3. ถ้า refresh ไม่ได้ → ลองดูมี API Key หรือไม่
4. ถ้าไม่มีอะไรเลย → return null

---

*สร้างโดยมินนี่ 🎀 — 16 เมษายน 2569*
