diff --git a/chrome/browser/about_flags.cc b/chrome/browser/about_flags.cc
index f90bc9600e1ece2aad88484638096b3ed451d3e2..04dd6bad4ddcc18634af814c80412e2ef6339447 100644
--- a/chrome/browser/about_flags.cc
+++ b/chrome/browser/about_flags.cc
@@ -362,8 +362,8 @@
 
 #if BUILDFLAG(IS_CHROMEOS) || BUILDFLAG(IS_LINUX) || BUILDFLAG(IS_MAC) || \
     BUILDFLAG(IS_WIN) || BUILDFLAG(IS_ANDROID)
-#include "chrome/browser/ui/webui/new_tab_page/composebox/variations/composebox_fieldtrial.h"  // nogncheck
 #include "chrome/browser/glic/suggestions/contextual_cueing_features.h"  // nogncheck
+#include "chrome/browser/ui/webui/new_tab_page/composebox/variations/composebox_fieldtrial.h"  // nogncheck
 #endif  // BUILDFLAG(IS_CHROMEOS) || BUILDFLAG(IS_LINUX) || BUILDFLAG(IS_MAC) ||
         // BUILDFLAG(IS_WIN) || BUILDFLAG(IS_ANDROID)
 
@@ -799,7 +799,6 @@ const FeatureEntry::Choice kReaderModeHeuristicsChoices[] = {
      switches::reader_mode_heuristics::kAllArticles},
 };
 
-
 const FeatureEntry::FeatureParam kReaderModeUseReadabilityDistiller[] = {
     {"use_distiller", "true"}};
 const FeatureEntry::FeatureParam kReaderModeUseReadabilityHeuristic[] = {
@@ -959,18 +958,21 @@ const FeatureEntry::FeatureParam kWebUIOmniboxDynamicAiModeButton_AnimOnly[] = {
     {"Omnibox_DynamicAnimation", "true"},
     {"Omnibox_DynamicColorScheme", "false"}};
 
-const FeatureEntry::FeatureParam kWebUIOmniboxDynamicAiModeButton_ColorOnly[] = {
-    {"Omnibox_DynamicAnimation", "false"},
-    {"Omnibox_DynamicColorScheme", "true"}};
+const FeatureEntry::FeatureParam kWebUIOmniboxDynamicAiModeButton_ColorOnly[] =
+    {{"Omnibox_DynamicAnimation", "false"},
+     {"Omnibox_DynamicColorScheme", "true"}};
 
 const FeatureEntry::FeatureParam kWebUIOmniboxDynamicAiModeButton_Both[] = {
     {"Omnibox_DynamicAnimation", "true"},
     {"Omnibox_DynamicColorScheme", "true"}};
 
-const FeatureEntry::FeatureVariation kWebUIOmniboxDynamicAiModeButtonVariations[] = {
-    {"Animation Only", kWebUIOmniboxDynamicAiModeButton_AnimOnly, nullptr},
-    {"Color Scheme Only", kWebUIOmniboxDynamicAiModeButton_ColorOnly, nullptr},
-    {"Animation and Color Scheme", kWebUIOmniboxDynamicAiModeButton_Both, nullptr}};
+const FeatureEntry::FeatureVariation
+    kWebUIOmniboxDynamicAiModeButtonVariations[] = {
+        {"Animation Only", kWebUIOmniboxDynamicAiModeButton_AnimOnly, nullptr},
+        {"Color Scheme Only", kWebUIOmniboxDynamicAiModeButton_ColorOnly,
+         nullptr},
+        {"Animation and Color Scheme", kWebUIOmniboxDynamicAiModeButton_Both,
+         nullptr}};
 
 const FeatureEntry::FeatureParam kOmniboxDynamicAimSubmitRhsHint[] = {
     {"Omnibox_ShowRhsAimHint", "true"},
@@ -1398,8 +1400,8 @@ const FeatureEntry::FeatureParam
         {"enable_tab_deselection", "true"}};
 
 // Normal 'Enabled' option is just the flag enabled with param 'realbox closes
-// menu on tab select' enabled by default. 'Disabled' option disables the flag, and thus
-// the context menu and 'realbox closes menu on tab select'.
+// menu on tab select' enabled by default. 'Disabled' option disables the flag,
+// and thus the context menu and 'realbox closes menu on tab select'.
 const FeatureEntry::FeatureVariation
     kContextManagementInComposeboxVariations[] = {
         {"Context Management in composebox (realbox closes menu on tab select)",
@@ -2046,7 +2048,6 @@ const FeatureEntry::FeatureParam kOmniboxMultilineEditFieldForAutocomplete[] = {
 const FeatureEntry::FeatureVariation kOmniboxMultilineEditFieldVariants[] = {
     {"For Autocomplete", kOmniboxMultilineEditFieldForAutocomplete, nullptr}};
 
-
 #endif  // BUILDFLAG(IS_ANDROID)
 
 const FeatureEntry::FeatureParam kMaxZeroSuggestMatches5[] = {
@@ -3914,8 +3915,6 @@ const FeatureEntry::FeatureVariation
          nullptr},
 };
 
-
-
 #if BUILDFLAG(IS_ANDROID)
 const FeatureEntry::FeatureParam kCCTResetTimeoutParams_1min[] = {
     {"minimum_reset_timeout_mins", "1"},
@@ -8553,7 +8552,6 @@ const FeatureEntry kFeatureEntries[] = {
      FEATURE_VALUE_TYPE(printing::features::kEnableOopPrintDrivers)},
 #endif
 
-
 #if BUILDFLAG(IS_ANDROID)
     {"incognito-screenshot", flag_descriptions::kIncognitoScreenshotName,
      flag_descriptions::kIncognitoScreenshotDescription, kOsAndroid,
@@ -10427,7 +10425,6 @@ const FeatureEntry kFeatureEntries[] = {
          kPermissionsGestureGatedPromptsVariations,
          "PermissionsGestureGatedPrompts")},
 
-
 #if !BUILDFLAG(IS_ANDROID)
     {"enable-lens-overlay-translate-button",
      flag_descriptions::kLensOverlayTranslateButtonName,
@@ -10959,6 +10956,17 @@ const FeatureEntry kFeatureEntries[] = {
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
@@ -11464,7 +11472,6 @@ const FeatureEntry kFeatureEntries[] = {
 #endif  // BUILDFLAG(IS_WIN) || BUILDFLAG(IS_MAC) || BUILDFLAG(IS_LINUX) ||
         // BUILDFLAG(IS_CHROMEOS) || BUILDFLAG(IS_ANDROID)
 
-
 #if !BUILDFLAG(IS_ANDROID)
     {"lens-overlay-permission-bubble-alt",
      flag_descriptions::kLensOverlayPermissionBubbleAltName,
@@ -12281,7 +12288,6 @@ const FeatureEntry kFeatureEntries[] = {
      FEATURE_VALUE_TYPE(
          autofill::features::kAutofillDisableBnplCountryCheckForTesting)},
 
-
 #if BUILDFLAG(IS_ANDROID)
     {"xplat-synced-setup", flag_descriptions::kXplatSyncedSetupName,
      flag_descriptions::kXplatSyncedSetupDescription, kOsAndroid,
