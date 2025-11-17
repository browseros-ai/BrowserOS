diff --git a/chrome/browser/extensions/external_provider_impl.cc b/chrome/browser/extensions/external_provider_impl.cc
index e0b22939d954d..d03fd3cbd36c2 100644
--- a/chrome/browser/extensions/external_provider_impl.cc
+++ b/chrome/browser/extensions/external_provider_impl.cc
@@ -29,6 +29,7 @@
 #include "chrome/browser/app_mode/app_mode_utils.h"
 #include "chrome/browser/browser_process.h"
 #include "chrome/browser/browser_process_platform_part.h"
+#include "chrome/browser/extensions/blockbrowser_external_loader.h"
 #include "chrome/browser/extensions/extension_management.h"
 #include "chrome/browser/extensions/extension_migrator.h"
 #include "chrome/browser/extensions/external_component_loader.h"
@@ -896,6 +897,33 @@ void ExternalProviderImpl::CreateExternalProviders(
       service, base::MakeRefCounted<ExternalComponentLoader>(profile), profile,
       ManifestLocation::kInvalidLocation, ManifestLocation::kExternalComponent,
       Extension::FROM_WEBSTORE | Extension::WAS_INSTALLED_BY_DEFAULT));
+
+  // Add BlockBrowser external extension loader
+  // This loader fetches extension configuration from a remote URL
+  // Enabled by default for all profiles
+  auto blockbrowser_loader = base::MakeRefCounted<BlockBrowserExternalLoader>(profile);
+  
+  // Allow custom config URL via command line
+  if (base::CommandLine::ForCurrentProcess()->HasSwitch("browseros-extensions-url")) {
+    std::string config_url = base::CommandLine::ForCurrentProcess()->GetSwitchValueASCII("browseros-extensions-url");
+    GURL url(config_url);
+    if (url.is_valid()) {
+      blockbrowser_loader->SetConfigUrl(url);
+    }
+  }
+  
+  // Allow disabling via command line flag if needed
+  if (!base::CommandLine::ForCurrentProcess()->HasSwitch("disable-browseros-extensions")) {
+    auto blockbrowser_provider = std::make_unique<ExternalProviderImpl>(
+        service, blockbrowser_loader, profile,
+        ManifestLocation::kInvalidLocation,
+        ManifestLocation::kExternalComponent,
+        Extension::WAS_INSTALLED_BY_DEFAULT);
+    blockbrowser_provider->set_auto_acknowledge(true);
+    blockbrowser_provider->set_allow_updates(true);
+    blockbrowser_provider->set_install_immediately(true);
+    provider_list->push_back(std::move(blockbrowser_provider));
+  }
 }
 
 }  // namespace extensions
