diff --git a/chrome/browser/ui/toolbar/toolbar_pref_names.cc b/chrome/browser/ui/toolbar/toolbar_pref_names.cc
index d93b933b81c6773b5f162c68ec9eba1fc0c3852f..85438f343f2eb3ea3456540d9eb72f740a49d98d 100644
--- a/chrome/browser/ui/toolbar/toolbar_pref_names.cc
+++ b/chrome/browser/ui/toolbar/toolbar_pref_names.cc
@@ -4,8 +4,10 @@
 
 #include "chrome/browser/ui/toolbar/toolbar_pref_names.h"
 
+#include "base/feature_list.h"
 #include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/ui_features.h"
+#include "chrome/common/chrome_features.h"
 #include "components/pref_registry/pref_registry_syncable.h"
 #include "components/prefs/pref_registry_simple.h"
 #include "ui/actions/actions.h"
@@ -14,14 +16,7 @@ namespace toolbar {
 
 void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry) {
   base::ListValue default_pinned_actions;
-  const std::optional<std::string>& chrome_labs_action =
-      actions::ActionIdMap::ActionIdToString(kActionShowChromeLabs);
-  // ActionIdToStringMappings are not initialized in unit tests, therefore will
-  // not have a value. In the normal case, the action should always have a
-  // value.
-  if (chrome_labs_action.has_value()) {
-    default_pinned_actions.Append(chrome_labs_action.value());
-  }
+  // Chrome Labs is no longer pinned by default
 
   if (base::FeatureList::IsEnabled(
           features::kTabsFromOtherDevicesSidePanelPinnedByDefault)) {
@@ -33,6 +28,15 @@ void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry) {
     }
   }
 
+  // Add third-party LLM panel to default pinned actions
+  if (base::FeatureList::IsEnabled(features::kThirdPartyLlmPanel)) {
+    const std::optional<std::string>& third_party_llm_action =
+        actions::ActionIdMap::ActionIdToString(kActionSidePanelShowThirdPartyLlm);
+    if (third_party_llm_action.has_value()) {
+      default_pinned_actions.Append(third_party_llm_action.value());
+    }
+  }
+
   registry->RegisterListPref(prefs::kPinnedActions,
                              std::move(default_pinned_actions),
                              user_prefs::PrefRegistrySyncable::SYNCABLE_PREF);
@@ -47,6 +51,9 @@ void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry) {
       user_prefs::PrefRegistrySyncable::SYNCABLE_PREF);
   registry->RegisterBooleanPref(prefs::kTabsFromOtherDevicesAutoPinnedMigration,
                                 false);
+  registry->RegisterBooleanPref(
+      prefs::kPinnedThirdPartyLlmMigrationComplete, false,
+      user_prefs::PrefRegistrySyncable::SYNCABLE_PREF);
 }
 
 }  // namespace toolbar
