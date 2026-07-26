diff --git a/chrome/browser/browseros/core/browseros_prefs.cc b/chrome/browser/browseros/core/browseros_prefs.cc
new file mode 100644
index 0000000000000..d1e1eea1df751
--- /dev/null
+++ b/chrome/browser/browseros/core/browseros_prefs.cc
@@ -0,0 +1,128 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/core/browseros_prefs.h"
+
+#include "chrome/browser/browseros/core/browseros_constants.h"
+#include "chrome/browser/browseros/core/browseros_product.h"
+#include "chrome/browser/ui/actions/chrome_action_id.h"
+#include "chrome/common/pref_names.h"
+#include "components/bookmarks/common/bookmark_pref_names.h"
+#include "components/pref_registry/pref_registry_syncable.h"
+#include "third_party/skia/include/core/SkColor.h"
+#include "ui/base/mojom/themes.mojom.h"
+
+namespace browseros {
+
+void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry) {
+  const bool show_toolbar_controls_by_default = !IsBrowserClawProduct();
+  const bool show_tab_groups_in_bookmark_bar_by_default =
+      !IsBrowserClawProduct();
+
+  registry->RegisterBooleanPref(prefs::kShowLLMChat,
+                                show_toolbar_controls_by_default);
+  registry->RegisterBooleanPref(prefs::kShowAssistant,
+                                show_toolbar_controls_by_default);
+  registry->RegisterBooleanPref(prefs::kShowToolbarLabels,
+                                show_toolbar_controls_by_default);
+  registry->RegisterBooleanPref(prefs::kVerticalTabsEnabled, true);
+  registry->RegisterBooleanPref(prefs::kShowTabGroupsInBookmarkBar,
+                                show_tab_groups_in_bookmark_bar_by_default);
+
+  registry->RegisterStringPref(prefs::kProviders, "");
+  registry->RegisterStringPref(prefs::kCustomProviders, "[]");
+  registry->RegisterStringPref(prefs::kDefaultProviderId, "");
+
+  registry->RegisterBooleanPref(prefs::kNtpFocusContent, false);
+  registry->RegisterBooleanPref(prefs::kOnboardingCompleted, false);
+}
+
+bool ShouldShowLLMChat(PrefService* pref_service) {
+  return pref_service->GetBoolean(prefs::kShowLLMChat);
+}
+
+bool ShouldShowAssistant(PrefService* pref_service) {
+  return pref_service->GetBoolean(prefs::kShowAssistant);
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
+bool ShouldShowTabGroupsInBookmarkBar(PrefService* pref_service) {
+  return pref_service->GetBoolean(prefs::kShowTabGroupsInBookmarkBar);
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
+void ApplyShowTabGroupsInBookmarkBarPref(PrefService* pref_service) {
+  pref_service->SetBoolean(bookmarks::prefs::kShowTabGroupsInBookmarkBar,
+                           ShouldShowTabGroupsInBookmarkBar(pref_service));
+}
+
+void SyncShowTabGroupsInBookmarkBarPref(PrefService* pref_service) {
+  const bool browseros_enabled = ShouldShowTabGroupsInBookmarkBar(pref_service);
+  const PrefService::Preference* upstream_pref = pref_service->FindPreference(
+      bookmarks::prefs::kShowTabGroupsInBookmarkBar);
+  if (upstream_pref && upstream_pref->IsDefaultValue() &&
+      pref_service->GetBoolean(bookmarks::prefs::kShowTabGroupsInBookmarkBar) !=
+          browseros_enabled) {
+    ApplyShowTabGroupsInBookmarkBarPref(pref_service);
+  }
+}
+
+void SyncDefaultTheme(PrefService* pref_service) {
+  const PrefService::Preference* user_color_pref =
+      pref_service->FindPreference(::prefs::kUserColor);
+  if (user_color_pref && user_color_pref->IsDefaultValue()) {
+    pref_service->SetInteger(::prefs::kUserColor,
+                             static_cast<int>(SkColorSetRGB(136, 136, 136)));
+    pref_service->SetString(::prefs::kCurrentThemeID, "user_color_theme_id");
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
+    case kActionBrowserOSAgent:
+      return prefs::kShowAssistant;
+    default:
+      return nullptr;
+  }
+}
+
+bool ShouldShowToolbarAction(actions::ActionId id, PrefService* pref_service) {
+  const char* pref_key = GetVisibilityPrefForAction(id);
+  if (!pref_key) {
+    return true;
+  }
+  return pref_service->GetBoolean(pref_key);
+}
+
+bool ShouldPinBrowserOSExtension(const std::string& extension_id,
+                                 PrefService*) {
+  return IsBrowserOSPinnedExtension(extension_id);
+}
+
+}  // namespace browseros
