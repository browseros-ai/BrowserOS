diff --git a/chrome/browser/prefs/browser_prefs.cc b/chrome/browser/prefs/browser_prefs.cc
index 48394ef7d6093b50a3c8906c2227c02ea7a6e5e8..853c0c836c86d8ea581709d1dbb64ead9482b924 100644
--- a/chrome/browser/prefs/browser_prefs.cc
+++ b/chrome/browser/prefs/browser_prefs.cc
@@ -24,6 +24,9 @@
 #include "chrome/browser/accessibility/page_colors_controller.h"
 #include "chrome/browser/accessibility/prefers_default_scrollbar_styles_prefs.h"
 #include "chrome/browser/browser_process_impl.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics_prefs.h"
+#include "chrome/browser/browseros/server/browseros_server_prefs.h"
 #include "chrome/browser/chrome_content_browser_client.h"
 #include "chrome/browser/component_updater/component_updater_prefs.h"
 #include "chrome/browser/contextual_cueing/prefs.h"
@@ -1397,6 +1400,8 @@ void RegisterLocalState(PrefRegistrySimple* registry) {
   breadcrumbs::RegisterPrefs(registry);
   browser_shutdown::RegisterPrefs(registry);
   BrowserProcessImpl::RegisterPrefs(registry);
+  browseros_server::RegisterLocalStatePrefs(registry);
+  browseros_metrics::RegisterLocalStatePrefs(registry);
   ChromeContentBrowserClient::RegisterLocalStatePrefs(registry);
 #if BUILDFLAG(CHROME_FOR_TESTING)
   chrome_for_testing::RegisterPrefs(registry);
@@ -1717,6 +1722,7 @@ void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry,
   AnnouncementNotificationService::RegisterProfilePrefs(registry);
   autofill::prefs::RegisterProfilePrefs(registry);
   browsing_data::prefs::RegisterBrowserUserPrefs(registry);
+  browseros_metrics::RegisterProfilePrefs(registry);
   capture_policy::RegisterProfilePrefs(registry);
   certificate_transparency::prefs::RegisterPrefs(registry);
   ChromeContentBrowserClient::RegisterProfilePrefs(registry);
@@ -1807,6 +1813,7 @@ void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry,
 #if !BUILDFLAG(IS_ANDROID)
   indigo::prefs::RegisterProfilePrefs(registry);
 #endif
+  RegisterBrowserOSPrefs(registry);
   RegisterPrefersDefaultScrollbarStylesPrefs(registry);
   RegisterSafetyHubProfilePrefs(registry);
 #if BUILDFLAG(IS_CHROMEOS)
@@ -2253,6 +2260,10 @@ void RegisterGeminiSettingsPrefs(user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterIntegerPref(prefs::kGeminiSettings, 0);
 }
 
+void RegisterBrowserOSPrefs(user_prefs::PrefRegistrySyncable* registry) {
+  browseros::RegisterProfilePrefs(registry);
+}
+
 #if BUILDFLAG(IS_CHROMEOS)
 void RegisterSigninProfilePrefs(user_prefs::PrefRegistrySyncable* registry,
                                 std::string_view country) {
@@ -2671,11 +2682,6 @@ void MigrateObsoleteProfilePrefs(PrefService* profile_prefs,
   syncer::ClearAccountKeyedPrefValue(
       profile_prefs, autofill::prefs::kAutofillAiOptInStatus, {});
 
-#if !BUILDFLAG(IS_ANDROID)
-  // Added 07/2026.
-  tabs::MigrateEverythingMenuPinnedToTabstripPref(profile_prefs);
-#endif
-
   // Please don't delete the following line. It is used by PRESUBMIT.py.
   // END_MIGRATE_OBSOLETE_PROFILE_PREFS
 
