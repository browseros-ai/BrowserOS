diff --git a/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc b/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc
index fb79fc34ddd781e9802524f5b41b5b55514895b2..4c21ff0f83860e744e91fb5be9b9ff7d52c2edc8 100644
--- a/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc
+++ b/chrome/browser/sync/prefs/chrome_syncable_prefs_database.cc
@@ -450,6 +450,9 @@ enum {
   kProfileDefaultContentSettingValuesInlineCueMenu = 100384,
   kExtensionsPinnedByDefault = 100385,
   kAudioFocusEnforcementEnabled = 100386,
+  // BrowserOS: keep the telemetry ID in sync with metadata/sync/enums.xml.
+  // The persisted/synced preference remains identified by its string name.
+  kPinnedThirdPartyLlmMigrationComplete = 100387,
   // See components/sync_preferences/README.md about adding new entries here.
   // vvvvv IMPORTANT! vvvvv
   // Note to the reviewer: IT IS YOUR RESPONSIBILITY to ensure that new syncable
@@ -654,6 +657,10 @@ constexpr auto kChromeSyncablePrefsAllowlist = base::MakeFixedFlatMap<
      {syncable_prefs_ids::kOrganizerPanelEntrypointEnabled, syncer::PREFERENCES,
       sync_preferences::PrefSensitivity::kNone,
       sync_preferences::MergeBehavior::kNone}},
+    {prefs::kPinnedThirdPartyLlmMigrationComplete,
+     {syncable_prefs_ids::kPinnedThirdPartyLlmMigrationComplete,
+      syncer::PREFERENCES, sync_preferences::PrefSensitivity::kNone,
+      sync_preferences::MergeBehavior::kNone}},
 #endif  // BUILDFLAG(IS_ANDROID)
 #if BUILDFLAG(ENABLE_EXTENSIONS_CORE)
     {extensions::pref_names::kPinnedExtensions,
