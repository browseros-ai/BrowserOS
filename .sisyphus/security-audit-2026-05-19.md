# BrowserOS Güvenlik Denetim Raporu

**Tarih**: 2026-05-19  
**Kapsam**: Tüm monorepo (packages/browseros-agent + packages/browseros)  
**Commit**: a59f96f6  
**Metod**: 8 paralel güvenlik tarama agentı ile otomatik statik analiz

---

## Yönetici Özeti

BrowserOS kod tabanında **7 kritik/yüksek**, **12 orta**, **8 düşük** seviyede güvenlik bulgusu tespit edildi. En kritik sorunlar:

1. **Tüm kimlik bilgileri düz metin olarak diskte saklanıyor** — API anahtarları, OAuth token'ları, konuşma geçmişi (şifreleme yok)
2. **`filesystem_bash` aracı sınırsız shell komutu çalıştırıyor** — sandbox, allowlist, onay mekanizması yok
3. **Tüm dosya sistemi araçlarında path traversal koruması yok** — `../../../etc/passwd` çalışıyor
4. **Kullanıcı konuşmaları ve ses kayıtları BrowserOS bulutuna yükleniyor** — kullanıcı farkında olmayabilir
5. **Kritik rotalar origin doğrulaması olmadan açık** — `/oauth`, `/mcp`, `/chat` korumasız

Hiçbir arka kapı veya gizli veri sızdırma mekanizması tespit edilmedi. Tüm ağ çağrıları meşru ürün işlevlerine hizmet ediyor.

---

## Bulgu Kataloğu

### 🔴 KRİTİK (7 bulgu)

#### C-1: API Anahtarları Düz Metin chrome.storage.local'da
**Dosya**: `apps/agent/lib/llm-providers/storage.ts:13`  
**Etkilenen veri**: OpenAI, Anthropic, Google, OpenRouter, Azure, Bedrock API anahtarları ve AWS kimlik bilgileri  
**Sorun**: `@wxt-dev/storage` (→ `chrome.storage.local`) hiçbir şifreleme katmanı içermiyor. `types.ts:37`'deki "encrypted and stored locally" yorumu **yanıltıcı**.  
**Risk**: Chrome profil dizinine erişimi olan herhangi bir işlem tüm API anahtarlarını okuyabilir.  
**Öneri**: Web Crypto API ile `crypto.subtle.encrypt()` kullanarak depolama öncesi şifreleme ekleyin.

#### C-2: OAuth Token'ları Düz Metin SQLite'da
**Dosya**: `apps/server/src/lib/db/schema/oauth.ts:21-22`  
**Etkilenen veri**: ChatGPT Pro/Plus, GitHub Copilot, Qwen Code için `access_token` ve `refresh_token`  
**Sorun**: SQLite `oauth_tokens` tablosu düz metin. `better-sqlite3` cipher uzantısı kullanılmıyor.  
**Risk**: `~/.browseros/db/browseros.sqlite` dosyasına erişen herkes token'ları çalabilir.  
**Öneri**: AES-256-GCM ile sütun seviyesinde şifreleme ekleyin.

#### C-3: `filesystem_bash` Sınırsız Shell Komutu Çalıştırıyor
**Dosya**: `apps/server/src/tools/filesystem/bash.ts:35-39`  
**Sorun**: `Bun.spawn([shell, '-c', params.command])` ile herhangi bir komut çalıştırılabilir. Allowlist yok, sandbox yok, environment kısıtlaması yok.  
**Risk**: Prompt injection yoluyla veya kötü niyetli LLM çıktısıyla `rm -rf /`, `curl evil.com`, `dd if=/dev/zero` gibi komutlar çalıştırılabilir.  
**Öneri**: Komut allowlist'i ekleyin, container içinde çalıştırın, tehlikeli komutları engelleyin.

#### C-4: Kullanıcı Konuşmaları BrowserOS Bulutuna Yükleniyor
**Dosya**: `apps/agent/lib/conversations/uploadConversationsToGraphql.ts`  
**Sorun**: Tüm sohbet mesajları (prompt'lar, yanıtlar, tool çağrıları) `api.browseros.com/graphql`'e senkronize ediliyor.  
**Risk**: README'deki "Your data never leaves your machine" iddiası ile çelişiyor. Kullanıcılar bu senkronizasyondan haberdar olmayabilir.  
**Öneri**: Açık kullanıcı onayı mekanizması ekleyin, privacy policy'de belirtin.

#### C-5: Build-Time Secret Inlining
**Dosya**: `scripts/build/server/compile.ts:34-42`  
**Sorun**: Production build'de `SENTRY_DSN`, `POSTHOG_API_KEY`, `BROWSEROS_CONFIG_URL` binary içine gömülüyor.  
**Risk**: `strings` komutu ile binary'den çıkarılabilir.  
**Öneri**: Bu değerleri runtime'da fetch edin veya environment variable'dan okuyun.

#### C-6: Tüm Dosya Sistemi Araçlarında Path Traversal
**Dosyalar**: `read.ts`, `write.ts`, `edit.ts`, `grep.ts`, `ls.ts`, `find.ts`, `framework.ts`, `page-actions.ts`  
**Sorun**: `path.resolve(cwd, params.path)` tüm araçlarda kullanılıyor ama sonucun workspace içinde kalıp kalmadığı kontrol edilmiyor.  
**Risk**: `../../../etc/passwd` okuyabilir, `/etc/cron.d/evil` yazabilir.  
**Öneri**: `resolve()` sonrası `relative()` ile workspace sınır kontrolü ekleyin.

#### C-7: Kritik Rotalarda Origin Doğrulaması Yok
**Dosya**: `apps/server/src/api/server.ts`  
**Sorun**: `/oauth`, `/klavis`, `/mcp`, `/chat` rotaları `requireTrustedAppOrigin()` middleware'i olmadan mount edilmiş. Server `0.0.0.0`'e bind oluyor.  
**Risk**: Aynı ağdaki herhangi biri OAuth token ekleyebilir, MCP komutu gönderebilir, LLM sağlayıcısını kullanabilir.  
**Öneri**: Ya bu rotalara `requireTrustedAppOrigin()` ekleyin, ya da default bind'i `127.0.0.1` yapın.

---

### 🟠 YÜKSEK (5 bulgu)

#### H-1: Arama Önerileri 5 Arama Motoruna Tuş Tuş Gönderiliyor
**Dosya**: `apps/agent/entrypoints/newtab/index/lib/searchSuggestions/getSearchSuggestions.ts`  
**Sorun**: Yeni sekmede yazılan her harf Google, Bing, Yahoo, DuckDuckGo, Brave'e aynı anda gönderiliyor.  
**Risk**: Kısmi arama sorguları 5 farklı şirkete sızıyor.  
**Öneri**: Sadece varsayılan arama motoruna gönderin veya opt-in yapın.

#### H-2: Sentry `sendDefaultPii: true`
**Dosya**: `apps/server/src/lib/sentry.ts:19`  
**Sorun**: Her hatada IP adresi ve request header'ları Sentry'ye gönderiliyor.  
**Öneri**: PII gönderimini kapatın veya sanitizasyon listesini genişletin.

#### H-3: Ses Kayıtları BrowserOS Bulutuna
**Dosya**: `apps/agent/lib/voice/transcribe-audio.ts`  
**Sorun**: Kullanıcı ses kayıtları `.webm` olarak `llm.browseros.com/api/transcribe`'a gönderiliyor.  
**Öneri**: Yerel transcription veya kullanıcı onayı.

#### H-4: Favicon İstekleri ile Gezinti Geçmişi Google'a Sızıyor
**Dosya**: `apps/agent/lib/getFavicons.ts`  
**Sorun**: Ziyaret edilen her sitenin domain'i `google.com/s2/favicons`'a gönderiliyor. DuckDuckGo alternatifi yorum satırına alınmış.  
**Öneri**: DuckDuckGo favicon servisine geri dönün veya privacy-preserving alternatif kullanın.

#### H-5: `codex-fetch.ts` BrowserOS Kullanıcılarını OpenAI'ye İşaretliyor
**Dosya**: `apps/server/src/lib/clients/oauth/codex-fetch.ts`  
**Sorun**: Tüm ChatGPT isteklerine `originator: browseros` header'ı ekleniyor, istekler `chatgpt.com/backend-api/codex`'e yönlendiriliyor.  
**Risk**: BrowserOS kullanıcıları OpenAI sistemlerinde tanımlanabilir hale geliyor.  
**Öneri**: Bu header'ın gerekliliğini değerlendirin, dokümante edin.

---

### 🟡 ORTA (12 bulgu)

| # | Bulgu | Dosya |
|---|-------|-------|
| M-1 | CORS overly permissive (`origin: '*'` + credentials) | `api/utils/cors.ts` |
| M-2 | SSRF riski: MCP transport probe URL doğrulaması zayıf | `lib/mcp-transport-detect.ts` |
| M-3 | ReDoS: grep aracında regex backtracking koruması yok | `tools/filesystem/grep.ts:133` |
| M-4 | URL injection: `javascript:` ve `file:` protokolleri engellenmiyor | `tools/navigation.ts`, `browser/browser.ts` |
| M-5 | `dangerouslySetInnerHTML` Shiki çıktısı ile kullanılıyor | `components/ai-elements/code-block.tsx` |
| M-6 | Container image'leri için imza doğrulaması yok | `lib/container/image-loader.ts` |
| M-7 | Container'larda kaynak limiti yok (CPU/memory/pids) | `lib/container/container-cli.ts` |
| M-8 | `zod-from-json-schema@0.1.0` — olgunlaşmamış paket | `apps/server/package.json` |
| M-9 | `chrome-devtools-mcp: "latest"` — versiyon sabitlenmemiş | `apps/server/package.json` |
| M-10 | OpenClaw gateway auth token'ı düz metin dosyada | `~/.openclaw/openclaw.json` |
| M-11 | JTBD anket verileri üçüncü parti Fly.io sunucusuna | `apps/agent/entrypoints/app/jtbd-agent/` |
| M-12 | Host-process agent'lar (Claude/Codex) container izolasyonu olmadan çalışıyor | `lib/agents/runtime/host-process-agent-runtime.ts` |

---

### 🟢 DÜŞÜK (8 bulgu)

- OAuth client ID'leri kaynak kodda (PKCE için normal)
- Test dosyalarında mock credential'lar (`sk-test` vb.)
- `upload_file` aracında path doğrulaması yok
- `/tmp/browseros-tool-output-*` temizlenmiyor
- Terminal WebSocket'te ek authentication yok
- MCP transport probe log'larında URL'ler görünüyor
- `.openclaw/` için `.gitignore` girişi yok
- `@types/bun: "latest"` eval paketinde sabitlenmemiş

---

## Hiçbir Arka Kapı veya Gizli Veri Sızdırma Tespit Edilmedi

Tüm ağ çağrıları meşru ürün işlevlerine hizmet ediyor:
- **LLM sağlayıcıları** (OpenAI, Anthropic, Google, vb.) — beklenen davranış
- **BrowserOS altyapısı** (api.browseros.com, llm.browseros.com, cdn.browseros.com)
- **Telemetri** (Sentry hata takibi, PostHog analitik)
- **Arama önerileri** (Google, Bing, Yahoo, DDG, Brave)
- **OAuth akışları** (standart PKCE/Device Code)

---

## Veri Akış Haritası

```
Kullanıcı Verisi → Nereye Gidiyor?
├── Konuşmalar → chrome.storage.local (düz metin) + api.browseros.com/graphql
├── API Anahtarları → chrome.storage.local (düz metin) + LLM sağlayıcıları
├── OAuth Token'ları → ~/.browseros/db/browseros.sqlite (düz metin)
├── Ses Kayıtları → llm.browseros.com/api/transcribe
├── Arama Sorguları → Google, Bing, Yahoo, DDG, Brave (5 motor)
├── Gezinti Verileri → Chromium profili + google.com/s2/favicons
├── Hata Raporları → Sentry (IP ve header'larla)
├── Kullanım Verileri → PostHog (tüm etkileşimler, session recording)
├── Bellek (Memory/Soul) → ~/.browseros/memory/*.md + ~/.browseros/SOUL.md
└── Dosya İşlemleri → Kullanıcının workspace dizini (path traversal riski var)
```

---

## Öncelikli Aksiyon Planı

### Faz 1 — Hemen (1-2 gün)
1. ✅ Kritik rotalara origin doğrulaması ekle (`/oauth`, `/mcp`, `/chat`)
2. ✅ Path traversal korumasını tüm dosya sistemi araçlarına ekle
3. ✅ `filesystem_bash` için minimum komut allowlist'i

### Faz 2 — Kısa Vade (1 hafta)
4. API anahtarları için Web Crypto API şifreleme
5. OAuth token'ları için AES-256-GCM şifreleme
6. `javascript:` ve `file:` URL protokollerini engelle
7. `sendDefaultPii: true` ayarını kapat veya sanitize et
8. `latest` ile sabitlenmiş paketleri pinle

### Faz 3 — Orta Vade (2-4 hafta)
9. Container image imza doğrulaması
10. Container kaynak limitleri
11. ReDoS koruması
12. Arama önerilerini sadece varsayılan motora indir
13. Favicon için privacy-preserving alternatif
14. Konuşma senkronizasyonu için kullanıcı onay mekanizması

### Faz 4 — Uzun Vade
15. `filesystem_bash` için container sandbox
16. Host-process agent'lar için container izolasyonu
17. CI/CD pipeline'a otomatik güvenlik taraması (gitleaks, npm audit, snyk)
18. Privacy policy güncellemesi — tüm veri akışlarını belgele

---

## Metodoloji

Bu denetim 8 paralel güvenlik tarama agentı ile gerçekleştirildi:

| Agent | Kapsam | Bulgu Sayısı |
|-------|--------|-------------|
| Ağ/Veri Sızdırma | Tüm outbound HTTP/WS çağrıları | 10 |
| Secret Yönetimi | API key, token, env var, kredansiyel | 10 |
| Tool Güvenliği | 60+ tool, sandbox, ACL, onay | 9 |
| Auth/OAuth/MCP | Kimlik doğrulama, OAuth, MCP entegrasyonları | 8 |
| Container/Docker | Konteyner, süreç, yetki yükseltme | 6 |
| Local Storage | Veri kalıcılığı, şifreleme, PII | 10 |
| Supply Chain | Bağımlılıklar, postinstall, registry | 3 |
| Injection | Command/Path/XSS/ReDoS/SQL injection | 12 |

**Toplam**: ~120 dosya incelendi, 32 bulgu kataloglandı.
