diff --git a/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.cc b/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.cc
index 613d124be1752..e33e71988598a 100644
--- a/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.cc
+++ b/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.cc
@@ -16,6 +16,7 @@
 #include "base/strings/strcat.h"
 #include "base/values.h"
 #include "chrome/browser/profiles/profile.h"
+#include "chrome/browser/ui/actions/browseros_actions_config.h"
 #include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/browser.h"
 #include "chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model_factory.h"
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
@@ -254,6 +258,23 @@ void PinnedToolbarActionsModel::MaybeMigrateExistingPinnedStates() {
   }
 }
 
+void PinnedToolbarActionsModel::EnsureAlwaysPinnedActions() {
+  // Only update if we're allowed to (not incognito/guest profiles).
+  if (!CanUpdate()) {
+    return;
+  }
+
+  // Pin native BlockBrowser actions if their features are enabled
+  for (actions::ActionId id : browseros::kBlockBrowserNativeActionIds) {
+    const base::Feature* feature = browseros::GetFeatureForBlockBrowserAction(id);
+    if (feature && base::FeatureList::IsEnabled(*feature) && !Contains(id)) {
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
