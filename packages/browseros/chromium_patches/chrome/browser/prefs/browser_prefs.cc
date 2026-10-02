diff --git a/chrome/browser/prefs/browser_prefs.cc b/chrome/browser/prefs/browser_prefs.cc
index 2653e37becbfa113d92456139d86c321f09ed8b5..ba5ae35d20cae30754a6f5fe44ced4a26641bbd0 100644
--- a/chrome/browser/prefs/browser_prefs.cc
+++ b/chrome/browser/prefs/browser_prefs.cc
@@ -24,6 +24,8 @@
 #include "chrome/browser/accessibility/page_colors_controller.h"
 #include "chrome/browser/accessibility/prefers_default_scrollbar_styles_prefs.h"
 #include "chrome/browser/browser_process_impl.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
+#include "chrome/browser/browseros/server/browseros_server_prefs.h"
 #include "chrome/browser/chrome_content_browser_client.h"
 #include "chrome/browser/component_updater/component_updater_prefs.h"
 #include "chrome/browser/context_hub/prefs.h"
@@ -1424,6 +1426,7 @@ void RegisterLocalState(PrefRegistrySimple* registry) {
   breadcrumbs::RegisterPrefs(registry);
   browser_shutdown::RegisterPrefs(registry);
   BrowserProcessImpl::RegisterPrefs(registry);
+  browseros_server::RegisterLocalStatePrefs(registry);
   ChromeContentBrowserClient::RegisterLocalStatePrefs(registry);
 #if BUILDFLAG(CHROME_FOR_TESTING)
   chrome_for_testing::RegisterPrefs(registry);
@@ -1852,6 +1855,7 @@ void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry,
 #if !BUILDFLAG(IS_ANDROID)
   indigo::prefs::RegisterProfilePrefs(registry);
 #endif
+  RegisterBrowserOSPrefs(registry);
   RegisterPrefersDefaultScrollbarStylesPrefs(registry);
   RegisterSafetyHubProfilePrefs(registry);
 #if BUILDFLAG(IS_CHROMEOS)
@@ -2307,6 +2311,10 @@ void RegisterGeminiSettingsPrefs(user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterIntegerPref(optimization_guide::prefs::kGeminiSettings, 0);
 }
 
+void RegisterBrowserOSPrefs(user_prefs::PrefRegistrySyncable* registry) {
+  browseros::RegisterProfilePrefs(registry);
+}
+
 #if BUILDFLAG(IS_CHROMEOS)
 void RegisterSigninProfilePrefs(user_prefs::PrefRegistrySyncable* registry,
                                 std::string_view country) {
