diff --git a/chrome/browser/about_flags.cc b/chrome/browser/about_flags.cc
index f477aa7fa23fdf04a23894117bcc727c0d924c90..63010bce47c8ff4ec6b03c9c82d88971cb27cae0 100644
--- a/chrome/browser/about_flags.cc
+++ b/chrome/browser/about_flags.cc
@@ -10921,6 +10921,18 @@ const FeatureEntry kFeatureEntries[] = {
      FEATURE_VALUE_TYPE(display::features::kFastDrmMasterDrop)},
 #endif  // BUILDFLAG(IS_CHROMEOS)
 
+#if !BUILDFLAG(IS_ANDROID)
+    {"enable-browseros-alpha-features",
+     flag_descriptions::kBrowserOsAlphaFeaturesName,
+     flag_descriptions::kBrowserOsAlphaFeaturesDescription, kOsDesktop,
+     FEATURE_VALUE_TYPE(features::kBrowserOsAlphaFeatures)},
+
+    {"enable-browseros-keyboard-shortcuts",
+     flag_descriptions::kBrowserOsKeyboardShortcutsName,
+     flag_descriptions::kBrowserOsKeyboardShortcutsDescription, kOsDesktop,
+     FEATURE_VALUE_TYPE(features::kBrowserOsKeyboardShortcuts)},
+#endif
+
 #if BUILDFLAG(IS_ANDROID)
     {"new-etc1-encoder", flag_descriptions::kNewEtc1EncoderName,
      flag_descriptions::kNewEtc1EncoderDescription, kOsAndroid,
