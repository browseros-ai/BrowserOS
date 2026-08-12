diff --git a/chrome/browser/ui/webui/settings/settings_localized_strings_provider.cc b/chrome/browser/ui/webui/settings/settings_localized_strings_provider.cc
index d21c249464c61a17e6087f65d0b18ecf090b1e5f..f035506fb25ff715fc573525d526c4f00b5ac89b 100644
--- a/chrome/browser/ui/webui/settings/settings_localized_strings_provider.cc
+++ b/chrome/browser/ui/webui/settings/settings_localized_strings_provider.cc
@@ -14,6 +14,7 @@
 #include "base/strings/escape.h"
 #include "base/strings/string_number_conversions.h"
 #include "base/strings/utf_string_conversions.h"
+#include "base/version_info/version_info.h"
 #include "build/branding_buildflags.h"
 #include "build/build_config.h"
 #include "build/buildflag.h"
@@ -345,6 +346,10 @@ void AddAboutStrings(content::WebUIDataSource* html_source, Profile* profile) {
   std::u16string browser_version = VersionUI::GetAnnotatedVersionStringForUi();
 
   html_source->AddString("aboutBrowserVersion", browser_version);
+  html_source->AddString(
+      "aboutBrowserOSVersion",
+      base::UTF8ToUTF16(
+          std::string(version_info::GetBrowserOSVersionNumber())));
   html_source->AddString(
       "aboutProductCopyright",
       base::i18n::MessageFormatter::FormatWithNumberedArgs(
@@ -1136,6 +1141,8 @@ void AddImportDataStrings(content::WebUIDataSource* html_source) {
       {"importCommit", IDS_SETTINGS_IMPORT_COMMIT},
       {"noProfileFound", IDS_SETTINGS_IMPORT_NO_PROFILE_FOUND},
       {"importSuccess", IDS_SETTINGS_IMPORT_SUCCESS},
+      {"importDialogExtensions", IDS_SETTINGS_IMPORT_EXTENSIONS_CHECKBOX},
+      {"importDialogCookies", IDS_SETTINGS_IMPORT_COOKIES_CHECKBOX},
   };
   html_source->AddLocalizedStrings(kLocalizedStrings);
 }
