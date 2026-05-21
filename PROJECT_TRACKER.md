# PROJECT_TRACKER.md - BrowserOS Katkıları

## Son Güncelleme: 2026-05-07

---

## ✅ Tamamlanan Görevler

### Issue #950 - Group Scheduled Task Results
**Status:** PR #961 OPEN (review bekliyor)

**Yapılan Değişiklikler:**
- `ScheduledTaskResultGroup.tsx` - Yeni accordion component
- `ScheduledTaskResults.tsx` - Düz liste → gruplanmış yapı
- `ScheduleResults.tsx` (newtab) - Tutarlılık güncellemesi
- `types.ts` - `groupRunsByJob()` helper fonksiyonu

**Copilot Review Comments (6/6 addressed):**
1. Running groups MAX_DISPLAY_COUNT'a takılıyordu → ✅ Düzeltildi
2. Jobs yüklenmeden önce boş state flash → ✅ Fallback eklendi
3. Nested button HTML issue → ✅ Mevcut pattern korundu
4. Duplike grouping logic → ✅ Helper çıkarıldı
5. Unused imports → ✅ Temizlendi
6. Screenshot eklendi → ✅

**CLA:** ✅ İmzalandı
**Screenshot:** ✅ PR'a eklendi

---

## 👀 Takip Edilenler

### Issue #926 - Delete/Clear Scheduled Task Runs
**Status:** PR #937 OPEN (A2rjav tarafından açılmış)

**Not:** #950 PR'ımız merge edildikten sonra rebase edilmeli. Merge conflict olabilir.

---

## 📋 Idea Backlog

- [ ] #925 - Flexible schedule options (cron-like)
- [ ] #927 - Scheduled task notifications
- [ ] #928 - Task execution history export
- [ ] #929 - Task grouping by category
- [ ] #930 - Task templates

---

## 🎯 Yarının Öncelikleri

1. **PR #961 Review** - Maintainer review bekleniyor
2. **PR #937 Rebase** - #950 merge edildikten sonra
3. **Yeni Issue** - Backlog'dan bir sonraki feature

---

## 📝 Notlar

- İlk open source katkı tamamlandı!
- BrowserOS monorepo yapısı: `packages/browseros-agent/` agent kodu
- Fork: `cenktekin/BrowserOS`
- Upstream: `browseros-ai/BrowserOS`

---

## 🔧 Teknik Notlar

### BrowserOS Agent Yapısı
```
packages/browseros-agent/
├── apps/agent/
│   ├── entrypoints/
│   │   ├── app/scheduled-tasks/  ← #950 burada
│   │   └── newtab/index/         ← ScheduleResults.tsx
│   └── lib/schedules/
│       └── scheduleStorage.ts    ← Hook'lar
```

### Commit Convention
`feat(agent): <description>` veya `fix(agent): <description>`

### Gerekenler
- `git-lfs` kurulu olmalı
- `bun install` ile dependency kurulumu
- `bun run codegen:agent` ile GraphQL codegen
