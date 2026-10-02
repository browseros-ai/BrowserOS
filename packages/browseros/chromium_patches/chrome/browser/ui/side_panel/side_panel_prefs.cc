diff --git a/chrome/browser/ui/side_panel/side_panel_prefs.cc b/chrome/browser/ui/side_panel/side_panel_prefs.cc
index dbb54fb99ff7feb2ed5260bca0b9980f269c16b8..fc1a151431a96fe5cf2e4beec3bf9b5e15c5a6eb 100644
--- a/chrome/browser/ui/side_panel/side_panel_prefs.cc
+++ b/chrome/browser/ui/side_panel/side_panel_prefs.cc
@@ -4,6 +4,7 @@
 
 #include "chrome/browser/ui/side_panel/side_panel_prefs.h"
 
+#include "base/feature_list.h"
 #include "base/i18n/rtl.h"
 #include "base/values.h"
 #include "build/build_config.h"
@@ -11,6 +12,7 @@
 #include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
 #include "chrome/browser/ui/browser_window/public/profile_browser_collection.h"
 #include "chrome/browser/ui/side_panel/side_panel_entry_id.h"
+#include "chrome/browser/ui/ui_features.h"
 #include "chrome/common/pref_names.h"
 #include "chrome/grit/generated_resources.h"
 #include "components/pref_registry/pref_registry_syncable.h"
@@ -27,6 +29,15 @@
 
 namespace side_panel_prefs {
 
+namespace {
+
+constexpr char kThirdPartyLlmProvidersPref[] =
+    "browseros.third_party_llm.providers";
+constexpr char kThirdPartyLlmSelectedProviderPref[] =
+    "browseros.third_party_llm.selected_provider";
+
+}  // namespace
+
 void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry) {
 // TODO(crbug.com/489780965): Move policies over as features are implemented.
 #if !BUILDFLAG(IS_ANDROID)
@@ -47,6 +58,11 @@ void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry) {
       base::i18n::IsRTL());
   registry->RegisterDictionaryPref(prefs::kSidePanelAlignmentOverrides,
                                    std::move(alignment_overrides));
+
+  if (base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel)) {
+    registry->RegisterListPref(kThirdPartyLlmProvidersPref);
+    registry->RegisterIntegerPref(kThirdPartyLlmSelectedProviderPref, 0);
+  }
 #endif
 }
 
