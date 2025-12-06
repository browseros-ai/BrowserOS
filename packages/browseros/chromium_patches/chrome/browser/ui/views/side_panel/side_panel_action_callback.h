diff --git a/chrome/browser/ui/views/side_panel/side_panel_action_callback.h b/chrome/browser/ui/views/side_panel/side_panel_action_callback.h
index 5387bddd4d417..2daabd0ac2242 100644
--- a/chrome/browser/ui/views/side_panel/side_panel_action_callback.h
+++ b/chrome/browser/ui/views/side_panel/side_panel_action_callback.h
@@ -5,8 +5,11 @@
 #ifndef CHROME_BROWSER_UI_VIEWS_SIDE_PANEL_SIDE_PANEL_ACTION_CALLBACK_H_
 #define CHROME_BROWSER_UI_VIEWS_SIDE_PANEL_SIDE_PANEL_ACTION_CALLBACK_H_
 
+#include <string>
+
 #include "chrome/browser/ui/views/side_panel/side_panel_entry_key.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_enums.h"
+#include "extensions/common/extension_id.h"
 #include "ui/actions/actions.h"
 #include "ui/base/class_property.h"
 
@@ -16,6 +19,13 @@ actions::ActionItem::InvokeActionCallback CreateToggleSidePanelActionCallback(
     SidePanelEntryKey key,
     BrowserWindowInterface* bwi);
 
+// Creates an action callback for BrowserOS extensions that uses the contextual
+// (tab-specific) side panel toggle, which auto-registers panel options per tab.
+actions::ActionItem::InvokeActionCallback
+CreateBrowserosToggleSidePanelActionCallback(
+    const extensions::ExtensionId& extension_id,
+    BrowserWindowInterface* bwi);
+
 extern const ui::ClassProperty<
     std::underlying_type_t<SidePanelOpenTrigger>>* const
     kSidePanelOpenTriggerKey;
