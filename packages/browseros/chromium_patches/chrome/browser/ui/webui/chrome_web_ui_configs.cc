diff --git a/chrome/browser/ui/webui/chrome_web_ui_configs.cc b/chrome/browser/ui/webui/chrome_web_ui_configs.cc
index 30dddd61a226f..21b4dfc68602c 100644
--- a/chrome/browser/ui/webui/chrome_web_ui_configs.cc
+++ b/chrome/browser/ui/webui/chrome_web_ui_configs.cc
@@ -53,6 +53,7 @@
 #include "chrome/browser/ui/webui/signin_internals_ui.h"
 #include "chrome/browser/ui/webui/sync_internals/sync_internals_ui.h"
 #include "chrome/browser/ui/webui/translate_internals/translate_internals_ui.h"
+#include "chrome/browser/ui/webui/browseros_welcome.h"
 #include "chrome/browser/ui/webui/usb_internals/usb_internals_ui.h"
 #include "chrome/browser/ui/webui/version/version_ui.h"
 #include "components/enterprise/buildflags/buildflags.h"
@@ -290,6 +291,7 @@ void RegisterChromeWebUIConfigs() {
   map.AddWebUIConfig(std::make_unique<SiteEngagementUIConfig>());
   map.AddWebUIConfig(std::make_unique<SyncInternalsUIConfig>());
   map.AddWebUIConfig(std::make_unique<TranslateInternalsUIConfig>());
+  map.AddWebUIConfig(std::make_unique<BrowserOSWelcomeUIConfig>());
   map.AddWebUIConfig(std::make_unique<UsbInternalsUIConfig>());
   map.AddWebUIConfig(std::make_unique<user_actions_ui::UserActionsUIConfig>());
   map.AddWebUIConfig(std::make_unique<VersionUIConfig>());
