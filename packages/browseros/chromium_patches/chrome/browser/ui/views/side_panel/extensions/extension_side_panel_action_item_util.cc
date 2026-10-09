diff --git a/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_action_item_util.cc b/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_action_item_util.cc
index 86b2acf28bf8fc2c05ff5decf7af9ef439036787..d7382a8a0182013de32c7986aae4e18ac85d6ec8 100644
--- a/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_action_item_util.cc
+++ b/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_action_item_util.cc
@@ -9,10 +9,13 @@
 
 #include "base/check_op.h"
 #include "base/strings/utf_string_conversions.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
+#include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/ui/browser_actions.h"
 #include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
 #include "chrome/browser/ui/side_panel/side_panel_action_callback.h"
 #include "chrome/browser/ui/side_panel/side_panel_entry.h"
+#include "chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.h"
 #include "extensions/common/extension.h"
 #include "ui/actions/actions.h"
 #include "ui/base/class_properties.h"
@@ -65,6 +68,17 @@ void AcquireActionItem(BrowserWindowInterface* browser,
                          std::underlying_type_t<actions::ActionPinnableState>(
                              actions::ActionPinnableState::kPinnable))
             .Build());
+
+    // All registered entries for this extension share one window action item.
+    // Apply the product pin policy when that item is created, so acquiring a
+    // second tab's reference does not repeat the profile preference update.
+    Profile* profile = browser->GetProfile();
+    if (browseros::ShouldPinBrowserOSExtension(extension.id(),
+                                              profile->GetPrefs())) {
+      if (auto* pinned_model = PinnedToolbarActionsModel::Get(profile)) {
+        pinned_model->UpdatePinnedState(action_id, true);
+      }
+    }
   }
   action_item->SetProperty(kReferenceCountKey,
                            action_item->GetProperty(kReferenceCountKey) + 1);
