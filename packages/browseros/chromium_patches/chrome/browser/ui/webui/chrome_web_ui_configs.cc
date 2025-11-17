diff --git a/chrome/browser/ui/webui/chrome_web_ui_configs.cc b/chrome/browser/ui/webui/chrome_web_ui_configs.cc
index cbcdde4afa71b..6b72a49ababc0 100644
--- a/chrome/browser/ui/webui/chrome_web_ui_configs.cc
+++ b/chrome/browser/ui/webui/chrome_web_ui_configs.cc
@@ -44,6 +44,7 @@
 #include "chrome/browser/ui/webui/signin_internals_ui.h"
 #include "chrome/browser/ui/webui/sync_internals/sync_internals_ui.h"
 #include "chrome/browser/ui/webui/translate_internals/translate_internals_ui.h"
+#include "chrome/browser/ui/webui/blockbrowser_first_run.h"
 #include "chrome/browser/ui/webui/usb_internals/usb_internals_ui.h"
 #include "chrome/browser/ui/webui/user_actions/user_actions_ui.h"
 #include "chrome/browser/ui/webui/version/version_ui.h"
@@ -77,6 +78,7 @@
 #include "chrome/browser/ui/webui/access_code_cast/access_code_cast_ui.h"
 #include "chrome/browser/ui/webui/app_service_internals/app_service_internals_ui.h"
 #include "chrome/browser/ui/webui/bookmarks/bookmarks_ui.h"
+#include "chrome/browser/ui/webui/clash_of_gpts/clash_of_gpts_ui.h"
 #include "chrome/browser/ui/webui/commerce/product_specifications_ui.h"
 #include "chrome/browser/ui/webui/commerce/shopping_insights_side_panel_ui.h"
 #include "chrome/browser/ui/webui/downloads/downloads_ui.h"
@@ -253,6 +255,7 @@ void RegisterChromeWebUIConfigs() {
   map.AddWebUIConfig(std::make_unique<SiteEngagementUIConfig>());
   map.AddWebUIConfig(std::make_unique<SyncInternalsUIConfig>());
   map.AddWebUIConfig(std::make_unique<TranslateInternalsUIConfig>());
+  map.AddWebUIConfig(std::make_unique<BlockBrowserFirstRunUIConfig>());
   map.AddWebUIConfig(std::make_unique<UsbInternalsUIConfig>());
   map.AddWebUIConfig(std::make_unique<UserActionsUIConfig>());
   map.AddWebUIConfig(std::make_unique<VersionUIConfig>());
@@ -287,6 +290,7 @@ void RegisterChromeWebUIConfigs() {
   map.AddWebUIConfig(std::make_unique<media_router::AccessCodeCastUIConfig>());
   map.AddWebUIConfig(std::make_unique<BookmarksSidePanelUIConfig>());
   map.AddWebUIConfig(std::make_unique<BookmarksUIConfig>());
+  map.AddWebUIConfig(std::make_unique<ClashOfGptsUIConfig>());
   map.AddWebUIConfig(std::make_unique<CustomizeChromeUIConfig>());
   map.AddWebUIConfig(std::make_unique<DownloadsUIConfig>());
   map.AddWebUIConfig(std::make_unique<FeedbackUIConfig>());
