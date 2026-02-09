diff --git a/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc b/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc
index cb73087d9476d..14a8d978660dc 100644
--- a/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc
+++ b/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc
@@ -436,6 +436,9 @@ enum {
   kPinContextualTaskButton = 100369,
   kAccessibilityReadAnythingOmniboxChipIgnoredCount = 100370,
   kAccessibilityReadAnythingLineFocus = 100371,
+  // BrowserOS: sync pref IDs
+  kPinnedThirdPartyLlmMigrationComplete = 100372,
+  kPinnedClashOfGptsMigrationComplete = 100373,
   // See components/sync_preferences/README.md about adding new entries here.
   // vvvvv IMPORTANT! vvvvv
   // Note to the reviewer: IT IS YOUR RESPONSIBILITY to ensure that new syncable
@@ -636,6 +639,14 @@ constexpr auto kChromeSyncablePrefsAllowlist = base::MakeFixedFlatMap<
      {syncable_prefs_ids::kVerticalTabsEnabled, syncer::PREFERENCES,
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
