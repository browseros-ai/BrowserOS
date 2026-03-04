diff --git a/chrome/browser/themes/theme_service.cc b/chrome/browser/themes/theme_service.cc
index 0c89bb8539af6..209620f3f0f9a 100644
--- a/chrome/browser/themes/theme_service.cc
+++ b/chrome/browser/themes/theme_service.cc
@@ -272,11 +272,11 @@ void ThemeService::RegisterProfilePrefs(
                                 SK_ColorTRANSPARENT);
   registry->RegisterIntegerPref(
       prefs::kDeprecatedBrowserColorSchemeDoNotUse,
-      static_cast<int>(ThemeService::BrowserColorScheme::kSystem),
+      static_cast<int>(ThemeService::BrowserColorScheme::kLight),
       user_prefs::PrefRegistrySyncable::SYNCABLE_PREF);
   registry->RegisterIntegerPref(
       prefs::kBrowserColorScheme,
-      static_cast<int>(ThemeService::BrowserColorScheme::kSystem));
+      static_cast<int>(ThemeService::BrowserColorScheme::kLight));
   registry->RegisterIntegerPref(
       prefs::kDeprecatedUserColorDoNotUse, SK_ColorTRANSPARENT,
       user_prefs::PrefRegistrySyncable::SYNCABLE_PREF);
