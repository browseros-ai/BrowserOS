diff --git a/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc b/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc
index 5ebd4dfda0200..ec77943380577 100644
--- a/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc
+++ b/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc
@@ -443,6 +443,9 @@ enum {
   kDesktopToiOSPriceTrackingPromoLastImpressionTimestamp = 100376,
   kDesktopToiOSPriceTrackingPromoImpressionsCounter = 100377,
   kDesktopToiOSPriceTrackingPromoOptOut = 100378,
+  // BrowserOS: sync pref IDs
+  kPinnedThirdPartyLlmMigrationComplete = 100379,
+  kPinnedClashOfGptsMigrationComplete = 100380,
   // See components/sync_preferences/README.md about adding new entries here.
   // vvvvv IMPORTANT! vvvvv
   // Note to the reviewer: IT IS YOUR RESPONSIBILITY to ensure that new syncable
@@ -643,6 +646,14 @@ constexpr auto kChromeSyncablePrefsAllowlist = base::MakeFixedFlatMap<
      {syncable_prefs_ids::kProjectsPanelEntrypointEnabled, syncer::PREFERENCES,
       sync_preferences::PrefSensitivity::kNone,
       sync_preferences::MergeBehavior::kNone}},
+    {prefs::kPinnedThirdPartyLlmMigrationComplete,
+     {syncable_prefs_ids::kPinnedThirdPartyLlmMigrationComplete, syncer::PREFERENCES,
+      sync_preferences::PrefSensitivity::kNone,
+      sync_preferences::MergeBehavior::kNone}},
+    {prefs::kPinnedClashOfGptsMigrationComplete,
+     {syncable_prefs_ids::kPinnedClashOfGptsMigrationComplete, syncer::PREFERENCES,
+      sync_preferences::PrefSensitivity::kNone,
+      sync_preferences::MergeBehavior::kNone}},
 #endif  // BUILDFLAG(IS_ANDROID)
 #if BUILDFLAG(ENABLE_EXTENSIONS_CORE)
     {extensions::pref_names::kPinnedExtensions,
