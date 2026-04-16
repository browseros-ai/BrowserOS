# วิธีตั้งค่า Z.AI GLM สำหรับ BrowserOS

## ภาพรวม

BrowserOS รองรับการใช้งาน Z.AI GLM เป็น Provider หลักสำหรับ LLM โดยมีรุ่นโมเดลที่รองรับ:

- **glm-5** - โมเดลล่าสุด พร้อมความสามารถทาง Reasoning สูง
- **glm-5-turbo** - โมเดลเร็ว สำหรับงานที่ต้องการความเร็ว
- **glm-4.7** - โมเดลที่คุ้มค่าต่อการใช้งาน
- **glm-4.7-flash** - โมเดลเร็วและประหยัด

## ขั้นตอนการตั้งค่า

### 1. รับ Z.AI API Key

1. เข้าไปที่ [Z.AI Console](https://open.bigmodel.cn/)
2. ล็อกอินหรือสมัครบัญชี
3. ไปที่หน้า [API Keys](https://open.bigmodel.cn/usercenter/apikeys)
4. กดปุ่ม "Create API Key" เพื่อสร้าง API Key ใหม่
5. คัดลอก API Key ที่ได้

### 2. ตั้งค่า Environment Variable

คัดลอกไฟล์ `.env.example` เป็น `.env`:

```bash
cp .env.example .env
```

แก้ไขไฟล์ `.env` โดยใส่ API Key ของคุณ:

```bash
ZAI_API_KEY=your_actual_zai_api_key_here
```

### 3. ติดตั้ง Dependencies

```bash
bun install
```

### 4. Build โปรเจกต์

```bash
bun run build
```

### 5. ตั้งค่า Provider ใน BrowserOS

1. เปิด BrowserOS Extension
2. ไปที่ Settings (ตั้งค่า)
3. ในส่วน LLM Providers ให้:
   - เลือก Provider: **Z.AI GLM**
   - Base URL: `https://open.bigmodel.cn/api/paas/v4/` (กรอกอัตโนมัติ)
   - Model ID: เลือก `glm-5`, `glm-5-turbo`, หรือ `glm-4.7`
   - API Key: วาง API Key ที่ได้จากขั้นตอนที่ 1
4. กด "Save" เพื่อบันทึกการตั้งค่า

## ข้อมูลโมเดลและราคา

| โมเดล | Context Window | Max Output | ราคา Input | ราคา Output |
|--------|----------------|------------|--------------|---------------|
| glm-5 | 202,752 tokens | 131,000 tokens | $1.00 / 1M tokens | $3.20 / 1M tokens |
| glm-4.7 | 204,800 tokens | 131,072 tokens | $0.60 / 1M tokens | $2.20 / 1M tokens |
| glm-4.7-flash | 200,000 tokens | 65,535 tokens | $0.07 / 1M tokens | $0.40 / 1M tokens |

## คุณสมบัติ

- ✅ รองรับ Text Generation
- ✅ รองรับ Tool Calls / Function Calling
- ✅ รองรับ Reasoning (Chain-of-Thought)
- ✅ รองรับ Thai Language ดีเยี่ยม
- ⚠️ ยังไม่รองรับ Images (สำหรับ glm-4.7 และ glm-5)

## การแก้ปัญหา (Troubleshooting)

### ข้อผิดพลาด: Authentication Failed

- ตรวจสอบว่า API Key ถูกต้องและยังใช้งานได้
- ตรวจสอบว่ามีเครดิตคงเหลือในบัญชี Z.AI

### ข้อผิดพลาด: Rate Limit Exceeded

- ลองใช้ glm-4.7-flash ที่มีราคาถูกกว่า
- รอสักครู่แล้วลองใหม่

### ข้อผิดพลาด: Connection Timeout

- ตรวจสอบการเชื่อมต่ออินเทอร์เน็ต
- ตรวจสอบว่า Base URL: `https://open.bigmodel.cn/api/paas/v4/` ถูกต้อง

## ลิงก์ที่เกี่ยวข้อง

- [Z.AI Official Website](https://open.bigmodel.cn/)
- [Z.AI API Documentation](https://open.bigmodel.cn/dev/api)
- [BrowserOS Documentation](https://docs.browseros.com/)

## การอัปเดต

หากมีโมเดลใหม่ๆ จาก Z.AI สามารถเพิ่มได้โดยแก้ไขไฟล์:
- `packages/browseros-agent/apps/agent/lib/llm-providers/providerTemplates.ts`
- `packages/browseros-agent/apps/agent/lib/llm-providers/models-dev-data.json`

---

**หมายเหตุ:** การใช้งาน API Key จะมีค่าใช้จ่ายตามการใช้งานจริง โปรดตรวจสอบ [Pricing Page](https://open.bigmodel.cn/pricing) ของ Z.AI สำหรับข้อมูลราคาที่อัปเดต
