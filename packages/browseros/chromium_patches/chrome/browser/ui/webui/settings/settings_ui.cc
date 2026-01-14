diff --git a/chrome/browser/ui/webui/settings/settings_ui.cc b/chrome/browser/ui/webui/settings/settings_ui.cc
index 56efa4ce3d88b..7e27434eb63f0 100644
--- a/chrome/browser/ui/webui/settings/settings_ui.cc
+++ b/chrome/browser/ui/webui/settings/settings_ui.cc
@@ -58,6 +58,7 @@
 #include "chrome/browser/ui/webui/settings/accessibility_main_handler.h"
 #include "chrome/browser/ui/webui/settings/appearance_handler.h"
 #include "chrome/browser/ui/webui/settings/browser_lifetime_handler.h"
+#include "chrome/browser/ui/webui/settings/browseros_metrics_handler.h"
 #include "chrome/browser/ui/webui/settings/downloads_handler.h"
 #include "chrome/browser/ui/webui/settings/font_handler.h"
 #include "chrome/browser/ui/webui/settings/hats_handler.h"
@@ -202,6 +203,8 @@ void SettingsUI::RegisterProfilePrefs(
   registry->RegisterBooleanPref(prefs::kImportDialogHistory, true);
   registry->RegisterBooleanPref(prefs::kImportDialogSavedPasswords, true);
   registry->RegisterBooleanPref(prefs::kImportDialogSearchEngine, true);
+  registry->RegisterBooleanPref(prefs::kImportDialogExtensions, true);
+  registry->RegisterBooleanPref(prefs::kImportDialogCookies, true);
 }
 
 SettingsUI::SettingsUI(content::WebUI* web_ui)
@@ -261,6 +264,7 @@ SettingsUI::SettingsUI(content::WebUI* web_ui)
 #if BUILDFLAG(IS_WIN) || BUILDFLAG(IS_MAC)
   AddSettingsPageUIHandler(std::make_unique<PasskeysHandler>());
 #endif
+  AddSettingsPageUIHandler(std::make_unique<BrowserOSMetricsHandler>());
 
 #if BUILDFLAG(IS_CHROMEOS)
   InitBrowserSettingsWebUIHandlers();
