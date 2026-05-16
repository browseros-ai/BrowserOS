diff --git a/chrome/browser/ui/browser_commands.h b/chrome/browser/ui/browser_commands.h
index 9a4b59415da4d..32e2bd6d3b645 100644
--- a/chrome/browser/ui/browser_commands.h
+++ b/chrome/browser/ui/browser_commands.h
@@ -20,6 +20,7 @@
 #include "chrome/browser/ui/tabs/tab_strip_model_delegate.h"
 #include "chrome/browser/ui/tabs/tab_strip_user_gesture_details.h"
 #include "components/split_tabs/split_tab_id.h"
+#include "components/split_tabs/split_tab_visual_data.h"
 #include "content/public/common/page_zoom.h"
 #include "printing/buildflags/buildflags.h"
 #include "ui/base/window_open_disposition.h"
@@ -169,8 +170,10 @@ void MoveGroupToExistingWindow(BrowserWindowInterface* source,
 void MuteSite(BrowserWindowInterface* browser);
 void PinTab(BrowserWindowInterface* browser);
 void GroupTab(BrowserWindowInterface* browser);
-void NewSplitTab(BrowserWindowInterface* browser,
-                 split_tabs::SplitTabCreatedSource source);
+void NewSplitTab(
+    BrowserWindowInterface* browser,
+    split_tabs::SplitTabCreatedSource source,
+    split_tabs::SplitTabLayout layout = split_tabs::SplitTabLayout::kVertical);
 
 // Tab group commands
 // These values are persisted to logs. Entries should not be renumbered
