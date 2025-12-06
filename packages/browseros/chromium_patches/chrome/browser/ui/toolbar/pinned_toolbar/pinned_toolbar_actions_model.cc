diff --git a/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.cc b/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.cc
index d9315fa4fa5b0..a82f97bad37d7 100644
--- a/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.cc
+++ b/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.cc
@@ -17,6 +17,7 @@
 #include "base/strings/strcat.h"
 #include "base/values.h"
 #include "chrome/browser/profiles/profile.h"
+#include "chrome/browser/ui/actions/browseros_actions_config.h"
 #include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model_factory.h"
 #include "chrome/browser/ui/toolbar/toolbar_pref_names.h"
@@ -236,8 +237,11 @@ void PinnedToolbarActionsModel::MaybeMigrateExistingPinnedStates() {
   if (!CanUpdate()) {
     return;
   }
+  // Chrome Labs is no longer automatically pinned for new profiles
+  // We keep this migration complete check to not affect users who already have
+  // it
   if (!pref_service_->GetBoolean(prefs::kPinnedChromeLabsMigrationComplete)) {
-    UpdatePinnedState(kActionShowChromeLabs, true);
+    // UpdatePinnedState(kActionShowChromeLabs, true);  // No longer auto-pin
     pref_service_->SetBoolean(prefs::kPinnedChromeLabsMigrationComplete, true);
   }
   if (features::HasTabSearchToolbarButton() &&
@@ -253,6 +257,25 @@ void PinnedToolbarActionsModel::MaybeMigrateExistingPinnedStates() {
   }
 }
 
+void PinnedToolbarActionsModel::EnsureAlwaysPinnedActions() {
+  // Only update if we're allowed to (not incognito/guest profiles).
+  if (!CanUpdate()) {
+    return;
+  }
+
+  // Pin native BrowserOS actions if their features are enabled (or no feature flag)
+  for (actions::ActionId id : browseros::kBrowserOSNativeActionIds) {
+    const base::Feature* feature = browseros::GetFeatureForBrowserOSAction(id);
+    // Pin if: no feature flag (always enabled) OR feature is enabled
+    bool should_pin = !feature || base::FeatureList::IsEnabled(*feature);
+    if (should_pin && !Contains(id)) {
+      UpdatePinnedState(id, true);
+    }
+  }
+  
+  // Note: Extension pinning is handled by ExtensionSidePanelManager
+}
+
 const std::vector<actions::ActionId>&
 PinnedToolbarActionsModel::PinnedActionIds() const {
   return pinned_action_ids_;
