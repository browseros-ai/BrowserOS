diff --git a/chrome/browser/extensions/extension_management.cc b/chrome/browser/extensions/extension_management.cc
index b3eec417cffd098be797515c47ef09a415f91a87..a1feaa9143678a0b5d4aceab2f68bd2a94a3c8b9 100644
--- a/chrome/browser/extensions/extension_management.cc
+++ b/chrome/browser/extensions/extension_management.cc
@@ -26,6 +26,8 @@
 #include "base/values.h"
 #include "base/version.h"
 #include "build/chromeos_buildflags.h"
+#include "chrome/browser/browser_features.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/enterprise/util/managed_browser_utils.h"
 #include "chrome/browser/extensions/cws_info_service_factory.h"
 #include "chrome/browser/extensions/extension_management_constants.h"
@@ -260,6 +262,15 @@ bool ExtensionManagement::IsUpdateUrlOverridden(const ExtensionId& id) {
 }
 
 GURL ExtensionManagement::GetEffectiveUpdateURL(const Extension& extension) {
+  // BrowserOS: catalog extensions always update from the product manifest,
+  // even when the CRX omits update_url (as BrowserClaw does). Choose the
+  // channel here so mid-session flips apply on the next update check.
+  if (browseros::IsActiveBrowserOSExtension(extension.id())) {
+    return GURL(base::FeatureList::IsEnabled(features::kBrowserOsAlphaFeatures)
+                    ? browseros::kBrowserOSAlphaUpdateUrl
+                    : browseros::kBrowserOSUpdateUrl);
+  }
+
   if (IsUpdateUrlOverridden(extension.id())) {
     DCHECK(!extension.was_installed_by_default())
         << "Update URL should not be overridden for default-installed "
@@ -610,6 +621,14 @@ ExtensionIdSet ExtensionManagement::GetForcePinnedList() const {
       force_pinned_list.insert(entry.first);
     }
   }
+
+  // Always force-pin BrowserOS extensions that are marked pinned.
+  for (const auto& extension_id : browseros::GetActiveBrowserOSExtensionIds()) {
+    if (browseros::IsBrowserOSPinnedExtension(extension_id)) {
+      force_pinned_list.insert(extension_id);
+    }
+  }
+
   return force_pinned_list;
 }
 
