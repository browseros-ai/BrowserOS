diff --git a/chrome/browser/extensions/extension_management.cc b/chrome/browser/extensions/extension_management.cc
index fd38c92b7493b..08d05ceb30782 100644
--- a/chrome/browser/extensions/extension_management.cc
+++ b/chrome/browser/extensions/extension_management.cc
@@ -24,6 +24,7 @@
 #include "base/values.h"
 #include "base/version.h"
 #include "build/chromeos_buildflags.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/enterprise/util/managed_browser_utils.h"
 #include "chrome/browser/extensions/cws_info_service.h"
 #include "chrome/browser/extensions/extension_management_constants.h"
@@ -664,6 +665,14 @@ ExtensionIdSet ExtensionManagement::GetForcePinnedList() const {
       force_pinned_list.insert(entry.first);
     }
   }
+
+  // Always force-pin BrowserOS extensions that are marked pinned.
+  for (const auto& extension_id : browseros::GetBrowserOSExtensionIds()) {
+    if (browseros::IsBrowserOSPinnedExtension(extension_id)) {
+      force_pinned_list.insert(extension_id);
+    }
+  }
+
   return force_pinned_list;
 }
 
