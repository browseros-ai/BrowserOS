diff --git a/chrome/browser/extensions/extension_management.cc b/chrome/browser/extensions/extension_management.cc
index fd38c92b7493b..a9349d7f9df30 100644
--- a/chrome/browser/extensions/extension_management.cc
+++ b/chrome/browser/extensions/extension_management.cc
@@ -9,6 +9,7 @@
 #include <utility>
 
 #include "base/command_line.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "base/containers/contains.h"
 #include "base/feature_list.h"
 #include "base/functional/bind.h"
@@ -362,31 +363,7 @@ bool ExtensionManagement::IsAllowedManifestVersion(
     int manifest_version,
     const std::string& extension_id,
     Manifest::Type manifest_type) {
-  bool enabled_by_default =
-      !base::FeatureList::IsEnabled(
-          extensions_features::kExtensionsManifestV3Only) ||
-      manifest_version >= 3;
-
-  // Manifest version policy only supports normal extensions and Chrome OS login
-  // screen extension.
-  if (manifest_type != Manifest::Type::TYPE_EXTENSION &&
-      manifest_type != Manifest::Type::TYPE_LOGIN_SCREEN_EXTENSION) {
-    return enabled_by_default;
-  }
-  switch (global_settings_->manifest_v2_setting) {
-    case internal::GlobalSettings::ManifestV2Setting::kDefault:
-      return enabled_by_default;
-    case internal::GlobalSettings::ManifestV2Setting::kDisabled:
-      return manifest_version >= 3;
-    case internal::GlobalSettings::ManifestV2Setting::kEnabled:
       return true;
-    case internal::GlobalSettings::ManifestV2Setting::kEnabledForForceInstalled:
-      auto installation_mode =
-          GetInstallationMode(extension_id, /*update_url=*/std::string());
-      return manifest_version >= 3 ||
-             installation_mode == ManagedInstallationMode::kForced ||
-             installation_mode == ManagedInstallationMode::kRecommended;
-  }
 }
 
 bool ExtensionManagement::IsAllowedManifestVersion(const Extension* extension) {
@@ -407,26 +384,8 @@ bool ExtensionManagement::IsExemptFromMV2DeprecationByPolicy(
     return false;
   }
 
-  switch (global_settings_->manifest_v2_setting) {
-    case internal::GlobalSettings::ManifestV2Setting::kDefault:
-      // Default browser behavior. Not exempt.
-      return false;
-    case internal::GlobalSettings::ManifestV2Setting::kDisabled:
-      // All MV2 extensions are disallowed. Not exempt.
-      return false;
-    case internal::GlobalSettings::ManifestV2Setting::kEnabled:
       // All MV2 extensions are allowed. Exempt.
       return true;
-    case internal::GlobalSettings::ManifestV2Setting::kEnabledForForceInstalled:
-      // Force-installed MV2 extensions are allowed. Exempt if it's a force-
-      // installed extension only.
-      auto installation_mode =
-          GetInstallationMode(extension_id, /*update_url=*/std::string());
-      return installation_mode == ManagedInstallationMode::kForced ||
-             installation_mode == ManagedInstallationMode::kRecommended;
-  }
-
-  return false;
 }
 
 bool ExtensionManagement::IsAllowedByUnpublishedAvailabilityPolicy(
@@ -664,6 +623,14 @@ ExtensionIdSet ExtensionManagement::GetForcePinnedList() const {
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
 
