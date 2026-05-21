# 🛡️ BrowserOS Security Hardening Log (Safkan-Secure)

Bu dosya, BrowserOS üzerindeki güvenlik açıklarını kapatmak için yapılan müdahalelerin kayıt defteridir. Kota dolması veya model değişimi durumunda sonraki agent buradan devam etmelidir.

## 🔱 Genel Strateji
1. **Rebase-First:** Upstream (`main`) güncellemeleri her zaman bu branch üzerine rebase edilir. (Not: Büyük mimari değişikliklerde manuel müdahale gerekebilir).
2. **Minimal Conflict:** Orijinal kod silinmez, wrapper veya interceptor pattern kullanılır.
3. **Dead-Code Telemetry:** Veri sızdıran fonksiyonlar silinmez, içleri `return` ile boşaltılır veya mock nesneler kullanılır.

## 📋 Mevcut Durum (2026-05-21)
**Audit Raporu:** `.sisyphus/security-audit-2026-05-19.md`

### ✅ Faz 1: Kanmayı Durdur (Tamamlandı)
- [x] **C-7: Origin Verification** (Server default host 127.0.0.1 yapıldı)
- [x] **C-6: Path Traversal Protection** (`resolveSafePath` eklendi ve tüm filesystem araçlarına entegre edildi)
- [x] **C-3: Bash Tool Sandbox** (Komut allowlist/forbidden list eklendi, pipeline ve redirection engellendi)

### ✅ Faz 2: Veri Sızıntılarının Kapatılması (Tamamlandı)
- [x] **C-4: Conversation Sync** (uploadConversationsToGraphql komple temizlendi)
- [x] **H-1: Search Suggestions** (getSearchSuggestions komple temizlendi)
- [x] **H-4: Favicon Leakage** (Google favicon servisi iptal edildi)
- [x] **H-3: Voice Recording Upload** (transcribe-audio.ts temizlendi)
- [x] **H-2: Sentry PII Leakage** (sendDefaultPii: false yapıldı)
- [x] **PostHog Analytics** (Session recording ve analitik mock object ile tamamen kapatıldı)

### ✅ Faz 3: Kriptografi (Tamamlandı)
- [x] **C-1: API Keys Encryption** (Web Crypto API + AES-GCM ile chrome.storage.local şifreleme eklendi)
- [x] **C-2: OAuth Tokens Encryption** (Node.js Crypto + AES-256-GCM ile SQLite şifreleme eklendi)
- [x] **Extra: Conversation History Encryption** (Konuşma geçmişi diskte artık tamamen şifreli saklanıyor)

---

## 🧪 Doğrulama (Validation)

### 2026-05-21: Checkpoint Build & Typecheck
- **bun run typecheck:** ✅ BAŞARILI. Tüm monorepo tip kontrolünden geçti.
- **Bileşen Uyumluluğu:** CLI (Go) ve Sunucu bağlantısı 127.0.0.1 ile uyumlu.
- **Kriptografi:** Server ve Agent tarafında şeffaf şifreleme katmanları başarıyla entegre edildi.

---

## 🛠️ Yapılan Müdahaleler

### 2026-05-21: Faz 1 & 2
(Detaylar önceki commitlerde mevcut)

### 2026-05-21: Faz 3 (Kriptografi)
1. **apps/server/src/lib/crypto.ts:** Node.js tabanlı AES-256-GCM modülü oluşturuldu.
2. **apps/server/src/lib/clients/oauth/token-store.ts:** OAuth token'ları veritabanına girmeden önce şifreleniyor.
3. **apps/agent/lib/crypto.ts:** Web Crypto API tabanlı AES-GCM modülü oluşturuldu.
4. **apps/agent/lib/llm-providers/storage.ts:** `providersStorage` sarmalanarak API key'lerin diskte şifreli durması sağlandı.
5. **apps/agent/lib/conversations/conversationStorage.ts:** Konuşma geçmişi şifreli depolamaya taşındı.
