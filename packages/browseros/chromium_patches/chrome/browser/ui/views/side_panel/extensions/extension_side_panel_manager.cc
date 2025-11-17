diff --git a/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc b/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc
index 30d4b3bc95d1c..558e8f442d671 100644
--- a/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc
+++ b/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc
@@ -6,6 +6,7 @@
 
 #include "base/memory/scoped_refptr.h"
 #include "base/strings/utf_string_conversions.h"
+#include "chrome/browser/extensions/blockbrowser_extension_constants.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/actions/chrome_actions.h"
@@ -13,6 +14,7 @@
 #include "chrome/browser/ui/browser_actions.h"
 #include "chrome/browser/ui/browser_finder.h"
 #include "chrome/browser/ui/browser_window/public/browser_window_features.h"
+#include "chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.h"
 #include "chrome/browser/ui/ui_features.h"
 #include "chrome/browser/ui/views/frame/browser_view.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_action_callback.h"
@@ -120,6 +122,14 @@ void ExtensionSidePanelManager::MaybeCreateActionItemForExtension(
                        std::underlying_type_t<actions::ActionPinnableState>(
                            actions::ActionPinnableState::kPinnable))
           .Build());
+
+  // Auto-pin BlockBrowser extensions to the toolbar.
+  if (browseros::IsBlockBrowserExtension(extension->id())) {
+    LOG(INFO) << "browseros: Auto-pinning BlockBrowser extension: " << extension->id();
+    if (auto* pinned_model = PinnedToolbarActionsModel::Get(profile_)) {
+      pinned_model->UpdatePinnedState(extension_action_id, true);
+    }
+  }
 }
 
 actions::ActionId ExtensionSidePanelManager::GetOrCreateActionIdForExtension(
@@ -159,6 +169,23 @@ void ExtensionSidePanelManager::OnExtensionUnloaded(
     it->second->DeregisterEntry();
     coordinators_.erase(extension->id());
   }
+  
+  // Unpin BlockBrowser extensions before removing the action item
+  if (browseros::IsBlockBrowserExtension(extension->id())) {
+    LOG(INFO) << "browseros: Unpinning BlockBrowser extension: " << extension->id() 
+              << " reason: " << static_cast<int>(reason);
+    if (auto* pinned_model = PinnedToolbarActionsModel::Get(profile_)) {
+      // Get the action ID to unpin it
+      std::optional<actions::ActionId> extension_action_id =
+          actions::ActionIdMap::StringToActionId(
+              SidePanelEntry::Key(SidePanelEntry::Id::kExtension, extension->id())
+                  .ToString());
+      if (extension_action_id.has_value()) {
+        pinned_model->UpdatePinnedState(extension_action_id.value(), false);
+      }
+    }
+  }
+  
   MaybeRemoveActionItemForExtension(extension);
 }
 
