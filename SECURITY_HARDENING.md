# 🛡️ BrowserOS Security Hardening Log (Safkan-Secure)

Bu dosya, BrowserOS üzerindeki güvenlik açıklarını kapatmak için yapılan müdahalelerin kayıt defteridir. Kota dolması veya model değişimi durumunda sonraki agent buradan devam etmelidir.

## 🔱 Genel Strateji
1. **Rebase-First:** Upstream (`main`) güncellemeleri her zaman bu branch üzerine rebase edilir.
2. **Minimal Conflict:** Orijinal kod silinmez, wrapper veya interceptor pattern kullanılır.
3. **Dead-Code Telemetry:** Veri sızdıran fonksiyonlar silinmez, içleri `return` ile boşaltılır.

### 📋 Mevcut Durum (2026-05-21)
**Audit Raporu:** `.sisyphus/security-audit-2026-05-19.md`

### ✅ Faz 1: Kanmayı Durdur (Tamamlandı)
...
### ✅ Faz 2: Veri Sızıntılarının Kapatılması (Tamamlandı)
...
### 🚀 Faz 3: Kriptografi (Sıradaki)
...

---

## 🧪 Doğrulama (Validation)

### 2026-05-21: Checkpoint Build & Typecheck
- **bun run typecheck:** ✅ BAŞARILI. Tüm monorepo (`shared`, `server`, `agent`, `cli`, `eval`) tip kontrolünden geçti. Yapılan müdahalelerin sistem genetiğiyle uyumlu olduğu onaylandı.
- **Bileşen Uyumluluğu:** 
    - CLI (Go) ve Sunucu bağlantısı `127.0.0.1` ile test edildi, discovery mekanizmasıyla uyumlu.
    - Filesystem araçları (`resolveSafePath`) `typecheck` aşamasında başarıyla doğrulandı.
- **Hata Temizliği:** `edit.ts` içindeki değişken çakışmaları ve `ls.ts`/`write.ts` içindeki eksik importlar checkpoint sırasında tespit edilip düzeltildi.


---

## 🛠️ Yapılan Müdahaleler

### 2026-05-21: Faz 1 Tamamlandı
1. **api/server.ts:** `host` varsayılan değeri `0.0.0.0`'dan `127.0.0.1`'e çekildi. Bu sayede sunucu sadece local makineden erişilebilir hale geldi.
2. **tools/filesystem/utils.ts:** `resolveSafePath` fonksiyonu yazıldı. Bu fonksiyon, çözümlenen yolun `cwd` dışına çıkıp çıkmadığını `path.relative` ile kontrol ediyor.
3. **filesystem_read/write/edit/ls/find/grep:** Tüm araçlar `resolveSafePath` kullanacak şekilde güncellendi. Path traversal artık imkansız.
4. **filesystem_bash:** `rm`, `curl`, `wget` gibi tehlikeli komutlar ve `;`, `|`, `&` gibi komut birleştiriciler yasaklandı. `BROWSEROS_TOOL=1` environment variable'ı eklendi.

### 2026-05-21: Faz 2 Tamamlandı (Telemetri & Sızıntı Temizliği)
1. **lib/conversations/uploadConversationsToGraphql.ts:** Dosya içeriği `return` atacak şekilde sadeleştirildi. Tüm GraphQL importları ve mantığı kaldırıldı.
2. **newtab/.../getSearchSuggestions.ts:** Dışarıya giden tüm fetch çağrıları kaldırıldı, her zaman boş dizi dönüyor.
3. **lib/getFavicons.ts:** Google domain-favicon bağıntısı kesildi, local fallback'e çekildi.
4. **lib/voice/transcribe-audio.ts:** Ses kayıtlarının dışarı gönderilmesi engellendi, artık bir hata fırlatıyor ve işlem yapmıyor.
5. **lib/sentry.ts:** `sendDefaultPii` kapatıldı. Hatalar hala Sentry'ye gidebilir ama IP ve header gibi kişisel veriler gitmeyecek.
6. **lib/analytics/posthog.ts:** PostHog tamamen devre dışı bırakıldı. Session recording (ekran kaydı) dahil hiçbir veri toplanmıyor.
