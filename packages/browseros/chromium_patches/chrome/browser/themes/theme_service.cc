diff --git a/chrome/browser/themes/theme_service.cc b/chrome/browser/themes/theme_service.cc
index cb1dd91e6d83b83cf1391f2d6215b10a9770731e..6d64c0c9d745fa2ab218c199f11552e8542804bc 100644
--- a/chrome/browser/themes/theme_service.cc
+++ b/chrome/browser/themes/theme_service.cc
@@ -34,6 +34,7 @@
 #include "base/trace_event/trace_event.h"
 #include "build/build_config.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
 #include "chrome/browser/extensions/extension_service.h"
 #include "chrome/browser/extensions/theme_installed_infobar_delegate.h"
 #include "chrome/browser/infobars/browser_infobar_manager.h"
@@ -312,11 +313,11 @@ void ThemeService::RegisterProfilePrefs(
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
@@ -355,6 +356,9 @@ ThemeService::~ThemeService() = default;
 void ThemeService::Init() {
   theme_helper_->DCheckCalledOnValidSequence();
 
+  // Seed the BrowserOS default before ThemeService builds its initial theme.
+  // SyncDefaultTheme only seeds the color while its preference is default.
+  browseros::SyncDefaultTheme(profile_->GetPrefs());
   InitFromPrefs();
 
   // ThemeObserver should be constructed before calling
