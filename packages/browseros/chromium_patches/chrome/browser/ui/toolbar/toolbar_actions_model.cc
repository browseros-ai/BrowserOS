diff --git a/chrome/browser/ui/toolbar/toolbar_actions_model.cc b/chrome/browser/ui/toolbar/toolbar_actions_model.cc
index 89c4445bf4185..39f22a4571f8e 100644
--- a/chrome/browser/ui/toolbar/toolbar_actions_model.cc
+++ b/chrome/browser/ui/toolbar/toolbar_actions_model.cc
@@ -18,6 +18,7 @@
 #include "base/one_shot_event.h"
 #include "base/strings/utf_string_conversions.h"
 #include "base/task/single_thread_task_runner.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/extension_management.h"
 #include "chrome/browser/extensions/extension_tab_util.h"
 #include "chrome/browser/extensions/managed_toolbar_pin_mode.h"
@@ -383,6 +384,11 @@ bool ToolbarActionsModel::IsActionPinned(const ActionId& action_id) const {
 }
 
 bool ToolbarActionsModel::IsActionForcePinned(const ActionId& action_id) const {
+  // Check if it's a BrowserOS extension
+  if (browseros::IsBrowserOSPinnedExtension(action_id)) {
+    return true;
+  }
+
   auto* management =
       extensions::ExtensionManagementFactory::GetForBrowserContext(profile_);
   return management->GetForcePinnedList().contains(action_id);
@@ -628,6 +634,14 @@ ToolbarActionsModel::GetFilteredPinnedActionIds() const {
                          return !std::ranges::contains(pinned, id);
                        });
 
+  // Add BrowserOS extensions to the force-pinned list (only those marked as pinned)
+  for (const std::string& ext_id : browseros::GetBrowserOSExtensionIds()) {
+    if (browseros::IsBrowserOSPinnedExtension(ext_id) &&
+        !std::ranges::contains(pinned, ext_id)) {
+      pinned.push_back(ext_id);
+    }
+  }
+
   // TODO(pbos): Make sure that the pinned IDs are pruned from ExtensionPrefs on
   // startup so that we don't keep saving stale IDs.
   std::vector<ActionId> filtered_action_ids;
