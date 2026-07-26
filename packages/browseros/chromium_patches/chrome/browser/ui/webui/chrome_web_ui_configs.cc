diff --git a/chrome/browser/ui/webui/chrome_web_ui_configs.cc b/chrome/browser/ui/webui/chrome_web_ui_configs.cc
index 30dddd61a226f..0941c08a125b4 100644
--- a/chrome/browser/ui/webui/chrome_web_ui_configs.cc
+++ b/chrome/browser/ui/webui/chrome_web_ui_configs.cc
@@ -7,6 +7,7 @@
 #include "build/android_buildflags.h"
 #include "build/branding_buildflags.h"
 #include "build/build_config.h"
+#include "chrome/browser/browseros/onboarding/browseros_onboarding.h"
 #include "chrome/browser/contextual_tasks/contextual_tasks_ui.h"
 #include "chrome/browser/glic/host/glic_ui.h"
 #include "chrome/browser/optimization_guide/optimization_guide_internals_ui.h"
@@ -290,6 +291,7 @@ void RegisterChromeWebUIConfigs() {
   map.AddWebUIConfig(std::make_unique<SiteEngagementUIConfig>());
   map.AddWebUIConfig(std::make_unique<SyncInternalsUIConfig>());
   map.AddWebUIConfig(std::make_unique<TranslateInternalsUIConfig>());
+  map.AddWebUIConfig(std::make_unique<BrowserOSOnboardingUIConfig>());
   map.AddWebUIConfig(std::make_unique<UsbInternalsUIConfig>());
   map.AddWebUIConfig(std::make_unique<user_actions_ui::UserActionsUIConfig>());
   map.AddWebUIConfig(std::make_unique<VersionUIConfig>());
