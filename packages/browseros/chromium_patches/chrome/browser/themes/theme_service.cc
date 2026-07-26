diff --git a/chrome/browser/themes/theme_service.cc b/chrome/browser/themes/theme_service.cc
index 17bdadbb26198..b73a85ace1945 100644
--- a/chrome/browser/themes/theme_service.cc
+++ b/chrome/browser/themes/theme_service.cc
@@ -30,6 +30,7 @@
 #include "base/task/thread_pool.h"
 #include "base/trace_event/trace_event.h"
 #include "build/build_config.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
 #include "chrome/browser/extensions/extension_service.h"
 #include "chrome/browser/extensions/theme_installed_infobar_delegate.h"
 #include "chrome/browser/new_tab_page/chrome_colors/chrome_colors_util.h"
@@ -273,11 +274,11 @@ void ThemeService::RegisterProfilePrefs(
                                 SK_ColorTRANSPARENT);
   registry->RegisterIntegerPref(
       prefs::kDeprecatedBrowserColorSchemeDoNotUse,
-      std::to_underlying(ThemeService::BrowserColorScheme::kSystem),
+      std::to_underlying(ThemeService::BrowserColorScheme::kLight),
       user_prefs::PrefRegistrySyncable::SYNCABLE_PREF);
   registry->RegisterIntegerPref(
       prefs::kBrowserColorScheme,
-      std::to_underlying(ThemeService::BrowserColorScheme::kSystem));
+      std::to_underlying(ThemeService::BrowserColorScheme::kLight));
   registry->RegisterIntegerPref(
       prefs::kDeprecatedUserColorDoNotUse, SK_ColorTRANSPARENT,
       user_prefs::PrefRegistrySyncable::SYNCABLE_PREF);
@@ -313,6 +314,7 @@ ThemeService::~ThemeService() = default;
 void ThemeService::Init() {
   theme_helper_->DCheckCalledOnValidSequence();
 
+  browseros::SyncDefaultTheme(profile_->GetPrefs());
   InitFromPrefs();
 
   // ThemeObserver should be constructed before calling
