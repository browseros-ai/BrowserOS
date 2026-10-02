diff --git a/chrome/browser/about_flags.cc b/chrome/browser/about_flags.cc
index 67e956fea7fd0e4e3e3bbf3ba30d47dbf5196901..4220fb26fe9b6d4ce5e6cd66232fac90620044de 100644
--- a/chrome/browser/about_flags.cc
+++ b/chrome/browser/about_flags.cc
@@ -11435,6 +11435,17 @@ const FeatureEntry kFeatureEntries[] = {
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
 #if BUILDFLAG(IS_ANDROID)
     {"new-etc1-encoder", flag_descriptions::kNewEtc1EncoderName,
      flag_descriptions::kNewEtc1EncoderDescription, kOsAndroid,
