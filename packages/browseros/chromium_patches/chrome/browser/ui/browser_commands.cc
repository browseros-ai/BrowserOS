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
