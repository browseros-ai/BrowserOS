diff --git a/chrome/browser/ui/webui/settings/settings_ui.cc b/chrome/browser/ui/webui/settings/settings_ui.cc
index ee6aaefb7fc82..67c338a86ccc8 100644
--- a/chrome/browser/ui/webui/settings/settings_ui.cc
+++ b/chrome/browser/ui/webui/settings/settings_ui.cc
@@ -59,6 +59,7 @@
 #include "chrome/browser/ui/webui/settings/accessibility_main_handler.h"
 #include "chrome/browser/ui/webui/settings/appearance_handler.h"
 #include "chrome/browser/ui/webui/settings/browser_lifetime_handler.h"
+#include "chrome/browser/ui/webui/settings/blockbrowser_metrics_handler.h"
 #include "chrome/browser/ui/webui/settings/downloads_handler.h"
 #include "chrome/browser/ui/webui/settings/font_handler.h"
 #include "chrome/browser/ui/webui/settings/hats_handler.h"
@@ -214,6 +215,7 @@ void SettingsUI::RegisterProfilePrefs(
   registry->RegisterBooleanPref(prefs::kImportDialogHistory, true);
   registry->RegisterBooleanPref(prefs::kImportDialogSavedPasswords, true);
   registry->RegisterBooleanPref(prefs::kImportDialogSearchEngine, true);
+  registry->RegisterBooleanPref(prefs::kImportDialogExtensions, true);
 }
 
 SettingsUI::SettingsUI(content::WebUI* web_ui)
@@ -291,6 +293,7 @@ SettingsUI::SettingsUI(content::WebUI* web_ui)
 #if BUILDFLAG(IS_WIN) || BUILDFLAG(IS_MAC)
   AddSettingsPageUIHandler(std::make_unique<PasskeysHandler>());
 #endif
+  AddSettingsPageUIHandler(std::make_unique<BlockBrowserMetricsHandler>());
 
 #if BUILDFLAG(IS_CHROMEOS)
   InitBrowserSettingsWebUIHandlers();
