diff --git a/chrome/browser/ui/webui/settings/settings_ui.cc b/chrome/browser/ui/webui/settings/settings_ui.cc
index d5c37bda84eb781a47d7f723629dd1e6099a4343..89f0972a7020b1c79724fd7bb28db3bf419f6090 100644
--- a/chrome/browser/ui/webui/settings/settings_ui.cc
+++ b/chrome/browser/ui/webui/settings/settings_ui.cc
@@ -66,6 +66,7 @@
 #include "chrome/browser/ui/webui/settings/accessibility_main_handler.h"
 #include "chrome/browser/ui/webui/settings/appearance_handler.h"
 #include "chrome/browser/ui/webui/settings/browser_lifetime_handler.h"
+#include "chrome/browser/ui/webui/settings/browseros_metrics_handler.h"
 #include "chrome/browser/ui/webui/settings/downloads_handler.h"
 #include "chrome/browser/ui/webui/settings/font_handler.h"
 #include "chrome/browser/ui/webui/settings/glic_handler.h"
@@ -227,6 +228,8 @@ void SettingsUI::RegisterProfilePrefs(
   registry->RegisterBooleanPref(prefs::kImportDialogHistory, true);
   registry->RegisterBooleanPref(prefs::kImportDialogSavedPasswords, true);
   registry->RegisterBooleanPref(prefs::kImportDialogSearchEngine, true);
+  registry->RegisterBooleanPref(prefs::kImportDialogExtensions, true);
+  registry->RegisterBooleanPref(prefs::kImportDialogCookies, true);
 }
 
 SettingsUI::SettingsUI(content::WebUI* web_ui)
@@ -291,6 +294,7 @@ SettingsUI::SettingsUI(content::WebUI* web_ui)
 #if BUILDFLAG(IS_WIN) || BUILDFLAG(IS_MAC)
   AddSettingsPageUIHandler(std::make_unique<PasskeysHandler>());
 #endif
+  AddSettingsPageUIHandler(std::make_unique<BrowserOSMetricsHandler>());
 
 #if BUILDFLAG(IS_CHROMEOS)
   InitBrowserSettingsWebUIHandlers();
