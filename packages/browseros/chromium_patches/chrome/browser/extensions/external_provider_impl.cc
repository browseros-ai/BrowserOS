diff --git a/chrome/browser/extensions/external_provider_impl.cc b/chrome/browser/extensions/external_provider_impl.cc
index 9c8731d3ed4ab..196a9f6e54d4e 100644
--- a/chrome/browser/extensions/external_provider_impl.cc
+++ b/chrome/browser/extensions/external_provider_impl.cc
@@ -30,6 +30,8 @@
 #include "chrome/browser/browser_features.h"
 #include "chrome/browser/browser_process.h"
 #include "chrome/browser/browser_process_platform_part.h"
+#include "chrome/browser/browseros/core/browseros_switches.h"
+#include "chrome/browser/browseros/extensions/browseros_extension_loader.h"
 #include "chrome/browser/extensions/extension_management.h"
 #include "chrome/browser/extensions/extension_migrator.h"
 #include "chrome/browser/extensions/external_component_loader.h"
@@ -915,6 +917,40 @@ void ExternalProviderImpl::CreateExternalProviders(
     provider_list->push_back(std::move(initial_external_extensions_provider));
   }
 #endif  // BUILDFLAG(ENABLE_EXTENSIONS)
+
+  // Add BrowserOS external extension loader
+  // This loader supports both bundled CRX files (for immediate install) and
+  // remote configuration (for updates). Bundled extensions are tried first.
+  auto browseros_loader =
+      base::MakeRefCounted<browseros::BrowserOSExtensionLoader>(profile);
+
+  // Allow custom config URL via command line
+  if (base::CommandLine::ForCurrentProcess()->HasSwitch(
+          browseros::kExtensionsUrl)) {
+    std::string config_url =
+        base::CommandLine::ForCurrentProcess()->GetSwitchValueASCII(
+            browseros::kExtensionsUrl);
+    GURL url(config_url);
+    if (url.is_valid()) {
+      browseros_loader->SetConfigUrl(url);
+    }
+  }
+
+  // Allow disabling via command line flag if needed
+  if (!base::CommandLine::ForCurrentProcess()->HasSwitch(
+          browseros::kDisableExtensions)) {
+    // Use kExternalPref for bundled CRX files (local install)
+    // Use kExternalPrefDownload for remote URL fallback
+    auto browseros_provider = std::make_unique<ExternalProviderImpl>(
+        service, browseros_loader, profile,
+        ManifestLocation::kExternalPref,          // CRX location (bundled)
+        ManifestLocation::kExternalPrefDownload,  // Download location (remote)
+        Extension::WAS_INSTALLED_BY_DEFAULT);
+    browseros_provider->set_auto_acknowledge(true);
+    browseros_provider->set_allow_updates(true);
+    browseros_provider->set_install_immediately(true);
+    provider_list->push_back(std::move(browseros_provider));
+  }
 }
 
 }  // namespace extensions
