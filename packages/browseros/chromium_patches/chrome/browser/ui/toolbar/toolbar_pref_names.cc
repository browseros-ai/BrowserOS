diff --git a/chrome/browser/ui/toolbar/toolbar_pref_names.cc b/chrome/browser/ui/toolbar/toolbar_pref_names.cc
index b0e7b69174614..7222762073203 100644
--- a/chrome/browser/ui/toolbar/toolbar_pref_names.cc
+++ b/chrome/browser/ui/toolbar/toolbar_pref_names.cc
@@ -4,8 +4,11 @@
 
 #include "chrome/browser/ui/toolbar/toolbar_pref_names.h"
 
+#include "base/feature_list.h"
 #include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/tab_search_feature.h"
+#include "chrome/browser/ui/ui_features.h"
+#include "chrome/common/chrome_features.h"
 #include "components/pref_registry/pref_registry_syncable.h"
 #include "components/prefs/pref_registry_simple.h"
 #include "ui/actions/actions.h"
@@ -14,14 +17,7 @@ namespace toolbar {
 
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
 
   if (features::HasTabSearchToolbarButton()) {
     const std::optional<std::string>& tab_search_action =
@@ -31,6 +27,24 @@ void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry) {
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
+  // Add Clash of GPTs panel to default pinned actions
+  if (base::FeatureList::IsEnabled(features::kClashOfGpts)) {
+    const std::optional<std::string>& clash_of_gpts_action =
+        actions::ActionIdMap::ActionIdToString(kActionSidePanelShowClashOfGpts);
+    if (clash_of_gpts_action.has_value()) {
+      default_pinned_actions.Append(clash_of_gpts_action.value());
+    }
+  }
+
   registry->RegisterListPref(prefs::kPinnedActions,
                              std::move(default_pinned_actions),
                              user_prefs::PrefRegistrySyncable::SYNCABLE_PREF);
@@ -46,6 +60,12 @@ void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterBooleanPref(
       prefs::kTabSearchMigrationComplete, false,
       user_prefs::PrefRegistrySyncable::SYNCABLE_PREF);
+  registry->RegisterBooleanPref(
+      prefs::kPinnedThirdPartyLlmMigrationComplete, false,
+      user_prefs::PrefRegistrySyncable::SYNCABLE_PREF);
+  registry->RegisterBooleanPref(
+      prefs::kPinnedClashOfGptsMigrationComplete, false,
+      user_prefs::PrefRegistrySyncable::SYNCABLE_PREF);
 }
 
 }  // namespace toolbar
