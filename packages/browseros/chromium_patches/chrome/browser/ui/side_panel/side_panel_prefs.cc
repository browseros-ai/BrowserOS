diff --git a/chrome/browser/ui/side_panel/side_panel_prefs.cc b/chrome/browser/ui/side_panel/side_panel_prefs.cc
index faccbffb9564bad21868de014c166498e6855cfc..5ed6fa7b255ac1c92bc91d4d71379dbec9a33a2e 100644
--- a/chrome/browser/ui/side_panel/side_panel_prefs.cc
+++ b/chrome/browser/ui/side_panel/side_panel_prefs.cc
@@ -4,6 +4,7 @@
 
 #include "chrome/browser/ui/side_panel/side_panel_prefs.h"
 
+#include "base/feature_list.h"
 #include "base/i18n/rtl.h"
 #include "base/values.h"
 #include "chrome/browser/profiles/profile.h"
@@ -11,6 +12,7 @@
 #include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
 #include "chrome/browser/ui/browser_window/public/profile_browser_collection.h"
 #include "chrome/browser/ui/side_panel/side_panel_entry_id.h"
+#include "chrome/browser/ui/ui_features.h"
 #include "chrome/common/pref_names.h"
 #include "chrome/grit/generated_resources.h"
 #include "components/pref_registry/pref_registry_syncable.h"
@@ -21,6 +23,15 @@
 
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
@@ -41,6 +52,11 @@ void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry) {
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
 
