diff --git a/chrome/browser/ui/browser_commands.cc b/chrome/browser/ui/browser_commands.cc
index cd7f650386e23..3ad70515264a1 100644
--- a/chrome/browser/ui/browser_commands.cc
+++ b/chrome/browser/ui/browser_commands.cc
@@ -119,6 +119,7 @@
 #include "chrome/browser/web_applications/web_app_helpers.h"
 #include "chrome/browser/web_applications/web_app_provider.h"
 #include "chrome/browser/web_applications/web_app_registrar.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/common/chrome_features.h"
 #include "chrome/common/content_restriction.h"
 #include "chrome/common/pref_names.h"
@@ -1479,7 +1480,8 @@ void GroupTab(BrowserWindowInterface* browser) {
 }
 
 void NewSplitTab(BrowserWindowInterface* browser,
-                 split_tabs::SplitTabCreatedSource source) {
+                 split_tabs::SplitTabCreatedSource source,
+                 split_tabs::SplitTabLayout layout) {
   TabStripModel* const tab_strip_model = browser->GetTabStripModel();
   const int active_index = tab_strip_model->active_index();
   // In Incognito mode, we can't show the regular Split View NTP so default to
@@ -1491,8 +1493,8 @@ void NewSplitTab(BrowserWindowInterface* browser,
       GURL(new_tab_url), active_index + 1, true,
       tab_strip_model->GetTabGroupForTab(active_index),
       tab_strip_model->IsTabPinned(active_index));
-  tab_strip_model->AddToNewSplit({active_index},
-                                 split_tabs::SplitTabVisualData(), source);
+  split_tabs::SplitTabVisualData visual_data(layout);
+  tab_strip_model->AddToNewSplit({active_index}, visual_data, source);
 }
 
 void AddNewTabToGroup(BrowserWindowInterface* browser) {
@@ -2528,7 +2529,20 @@ bool IsDebuggerAttachedToCurrentTab(BrowserWindowInterface* browser) {
 void CopyURL(BrowserWindowInterface* browser,
              content::WebContents* web_contents) {
   ui::ScopedClipboardWriter scw(ui::ClipboardBuffer::kCopyPaste);
-  scw.WriteText(base::UTF8ToUTF16(web_contents->GetVisibleURL().spec()));
+  GURL url = web_contents->GetVisibleURL();
+
+  // Transform BrowserOS extension URLs to virtual URLs for copying
+  if (url.SchemeIs(extensions::kExtensionScheme)) {
+    std::string virtual_url = browseros::GetBrowserOSVirtualURL(
+        url.host(), url.path(), url.ref());
+    if (!virtual_url.empty()) {
+      scw.WriteText(base::UTF8ToUTF16(virtual_url));
+    } else {
+      scw.WriteText(base::UTF8ToUTF16(url.spec()));
+    }
+  } else {
+    scw.WriteText(base::UTF8ToUTF16(url.spec()));
+  }
 
 #if !BUILDFLAG(IS_ANDROID)
   if (toast_features::IsEnabled(toast_features::kLinkCopiedToast)) {
