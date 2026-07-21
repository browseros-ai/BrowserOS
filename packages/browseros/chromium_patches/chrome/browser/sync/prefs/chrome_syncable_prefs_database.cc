diff --git a/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc b/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc
index 04c705f5a11d42672ce46ca50673002450053e68..b183751ca10266c41f3f0c4d6bdda8fbce0a1c5b 100644
--- a/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc
+++ b/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc
@@ -445,6 +445,8 @@ enum {
   kProfileContentSettingsExceptionsInlineCueMenu = 100382,
   kProfileContentSettingsPartitionedExceptionsInlineCueMenu = 100383,
   kProfileDefaultContentSettingValuesInlineCueMenu = 100384,
+  // BrowserOS: sync pref IDs
+  kPinnedThirdPartyLlmMigrationComplete = 100385,
   // See components/sync_preferences/README.md about adding new entries here.
   // vvvvv IMPORTANT! vvvvv
   // Note to the reviewer: IT IS YOUR RESPONSIBILITY to ensure that new syncable
@@ -649,6 +651,10 @@ constexpr auto kChromeSyncablePrefsAllowlist = base::MakeFixedFlatMap<
      {syncable_prefs_ids::kProjectsPanelEntrypointEnabled, syncer::PREFERENCES,
       sync_preferences::PrefSensitivity::kNone,
       sync_preferences::MergeBehavior::kNone}},
+    {prefs::kPinnedThirdPartyLlmMigrationComplete,
+     {syncable_prefs_ids::kPinnedThirdPartyLlmMigrationComplete, syncer::PREFERENCES,
+      sync_preferences::PrefSensitivity::kNone,
+      sync_preferences::MergeBehavior::kNone}},
 #endif  // BUILDFLAG(IS_ANDROID)
 #if BUILDFLAG(ENABLE_EXTENSIONS_CORE)
     {extensions::pref_names::kPinnedExtensions,
