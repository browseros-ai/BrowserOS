diff --git a/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc b/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc
index 30d4b3bc95d1c..20f8445279625 100644
--- a/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc
+++ b/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc
@@ -6,6 +6,7 @@
 
 #include "base/memory/scoped_refptr.h"
 #include "base/strings/utf_string_conversions.h"
+#include "chrome/browser/extensions/browseros_extension_constants.h"
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
@@ -20,6 +22,7 @@
 #include "chrome/browser/ui/views/side_panel/side_panel_registry.h"
 #include "content/public/browser/browser_context.h"
 #include "content/public/browser/web_contents.h"
+#include "extensions/browser/unloaded_extension_reason.h"
 #include "extensions/common/extension.h"
 #include "extensions/common/extension_features.h"
 #include "extensions/common/permissions/api_permission.h"
@@ -108,18 +111,41 @@ void ExtensionSidePanelManager::MaybeCreateActionItemForExtension(
 
   // Create a new action item.
   actions::ActionItem* root_action_item = browser_actions->root_action_item();
+  actions::ActionItem::InvokeActionCallback callback;
+  if (browseros::IsBrowserOSLabelledExtension(extension->id())) {
+    // For BrowserOS labelled extensions, check if it uses contextual toggle
+    if (browseros::UsesContextualSidePanelToggle(extension->id())) {
+      // Agent V2 uses contextual toggle that auto-registers tab-specific panel
+      callback = CreateBrowserosToggleSidePanelActionCallback(extension->id(),
+                                                              browser_);
+    } else {
+      // Other labelled extensions use the standard side panel toggle
+      callback = CreateToggleSidePanelActionCallback(
+          SidePanelEntry::Key(SidePanelEntry::Id::kExtension, extension->id()),
+          browser_);
+    }
+  } else {
+    callback = CreateToggleSidePanelActionCallback(
+        SidePanelEntry::Key(SidePanelEntry::Id::kExtension, extension->id()),
+        browser_);
+  }
+
   root_action_item->AddChild(
-      actions::ActionItem::Builder(
-          CreateToggleSidePanelActionCallback(
-              SidePanelEntry::Key(SidePanelEntry::Id::kExtension,
-                                  extension->id()),
-              browser_))
+      actions::ActionItem::Builder(std::move(callback))
           .SetText(base::UTF8ToUTF16(extension->short_name()))
           .SetActionId(extension_action_id)
           .SetProperty(actions::kActionItemPinnableKey,
                        std::underlying_type_t<actions::ActionPinnableState>(
                            actions::ActionPinnableState::kPinnable))
           .Build());
+
+  // Auto-pin BrowserOS extensions to the toolbar.
+  if (browseros::IsBrowserOSPinnedExtension(extension->id())) {
+    LOG(INFO) << "browseros: Auto-pinning BrowserOS extension: " << extension->id();
+    if (auto* pinned_model = PinnedToolbarActionsModel::Get(profile_)) {
+      pinned_model->UpdatePinnedState(extension_action_id, true);
+    }
+  }
 }
 
 actions::ActionId ExtensionSidePanelManager::GetOrCreateActionIdForExtension(
@@ -159,6 +185,26 @@ void ExtensionSidePanelManager::OnExtensionUnloaded(
     it->second->DeregisterEntry();
     coordinators_.erase(extension->id());
   }
+  
+  // Unpin BrowserOS labelled extensions only for permanent removal reasons.
+  // Don't unpin for UPDATE, TERMINATE, PROFILE_SHUTDOWN, etc. since the
+  // extension will come back.
+  bool should_unpin = (reason == UnloadedExtensionReason::DISABLE ||
+                       reason == UnloadedExtensionReason::UNINSTALL);
+  if (should_unpin && browseros::IsBrowserOSLabelledExtension(extension->id())) {
+    LOG(INFO) << "browseros: Unpinning BrowserOS extension: " << extension->id()
+              << " reason: " << static_cast<int>(reason);
+    if (auto* pinned_model = PinnedToolbarActionsModel::Get(profile_)) {
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
 
