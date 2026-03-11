diff --git a/chrome/browser/browseros/core/browseros_prefs.cc b/chrome/browser/browseros/core/browseros_prefs.cc
new file mode 100644
index 0000000000000..c191fb3963968
--- /dev/null
+++ b/chrome/browser/browseros/core/browseros_prefs.cc
@@ -0,0 +1,96 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/core/browseros_prefs.h"
+
+#include "chrome/browser/ui/actions/chrome_action_id.h"
+#include "chrome/common/pref_names.h"
+#include "components/pref_registry/pref_registry_syncable.h"
+#include "third_party/skia/include/core/SkColor.h"
+#include "ui/base/mojom/themes.mojom.h"
+
+namespace browseros {
+
+void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry) {
+  // Toolbar visibility prefs
+  registry->RegisterBooleanPref(prefs::kShowLLMChat, true);
+  registry->RegisterBooleanPref(prefs::kShowLLMHub, true);
+  registry->RegisterBooleanPref(prefs::kShowToolbarLabels, true);
+
+  // Vertical tabs pref
+  registry->RegisterBooleanPref(prefs::kVerticalTabsEnabled, true);
+
+  // AI Provider prefs
+  registry->RegisterStringPref(prefs::kProviders, "");
+  registry->RegisterStringPref(prefs::kCustomProviders, "[]");
+  registry->RegisterStringPref(prefs::kDefaultProviderId, "");
+
+  // NTP focus pref
+  registry->RegisterBooleanPref(prefs::kNtpFocusContent, false);
+}
+
+bool ShouldShowLLMChat(PrefService* pref_service) {
+  return pref_service->GetBoolean(prefs::kShowLLMChat);
+}
+
+bool ShouldShowLLMHub(PrefService* pref_service) {
+  return pref_service->GetBoolean(prefs::kShowLLMHub);
+}
+
+bool ShouldShowToolbarLabels(PrefService* pref_service) {
+  return pref_service->GetBoolean(prefs::kShowToolbarLabels);
+}
+
+bool IsVerticalTabsEnabled(PrefService* pref_service) {
+  return pref_service->GetBoolean(prefs::kVerticalTabsEnabled);
+}
+
+void SyncVerticalTabsPref(PrefService* pref_service) {
+  const bool browseros_enabled =
+      pref_service->GetBoolean(prefs::kVerticalTabsEnabled);
+  const PrefService::Preference* upstream_pref =
+      pref_service->FindPreference(::prefs::kVerticalTabsEnabled);
+  if (upstream_pref && upstream_pref->IsDefaultValue()) {
+    pref_service->SetBoolean(::prefs::kVerticalTabsEnabled, browseros_enabled);
+  }
+}
+
+void SyncDefaultTheme(PrefService* pref_service) {
+  const PrefService::Preference* user_color_pref =
+      pref_service->FindPreference(::prefs::kUserColor);
+  if (user_color_pref && user_color_pref->IsDefaultValue()) {
+    pref_service->SetInteger(::prefs::kUserColor,
+                             static_cast<int>(SkColorSetRGB(136, 136, 136)));
+    pref_service->SetString(::prefs::kCurrentThemeID,
+                            "user_color_theme_id");
+    pref_service->SetInteger(
+        ::prefs::kBrowserColorVariant,
+        static_cast<int>(ui::mojom::BrowserColorVariant::kNeutral));
+  }
+}
+
+bool IsNtpFocusContentEnabled(PrefService* pref_service) {
+  return pref_service->GetBoolean(prefs::kNtpFocusContent);
+}
+
+const char* GetVisibilityPrefForAction(actions::ActionId id) {
+  switch (id) {
+    case kActionSidePanelShowThirdPartyLlm:
+      return prefs::kShowLLMChat;
+    case kActionSidePanelShowClashOfGpts:
+      return prefs::kShowLLMHub;
+    default:
+      return nullptr;
+  }
+}
+
+bool ShouldShowToolbarAction(actions::ActionId id, PrefService* pref_service) {
+  const char* pref_key = GetVisibilityPrefForAction(id);
+  if (!pref_key) {
+    return true;  // No pref means always show
+  }
+  return pref_service->GetBoolean(pref_key);
+}
+
+}  // namespace browseros
