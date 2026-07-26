diff --git a/chrome/browser/ui/accelerator_table.cc b/chrome/browser/ui/accelerator_table.cc
index 06e3a9b5e0ee9..a49cac1c4052d 100644
--- a/chrome/browser/ui/accelerator_table.cc
+++ b/chrome/browser/ui/accelerator_table.cc
@@ -15,6 +15,7 @@
 #include "build/branding_buildflags.h"
 #include "build/build_config.h"
 #include "chrome/app/chrome_command_ids.h"
+#include "chrome/browser/browser_features.h"
 #include "chrome/browser/ui/tabs/features.h"
 #include "chrome/browser/ui/ui_features.h"
 #include "components/lens/buildflags.h"
@@ -333,6 +334,17 @@ std::vector<AcceleratorMapping> GetAcceleratorList() {
     }
 #endif
 
+    if (base::FeatureList::IsEnabled(features::kBrowserOsKeyboardShortcuts)) {
+      accelerators->push_back(
+          {ui::VKEY_K, ui::EF_SHIFT_DOWN | ui::EF_PLATFORM_ACCELERATOR,
+           IDC_SHOW_THIRD_PARTY_LLM_SIDE_PANEL});
+      accelerators->push_back(
+          {ui::VKEY_L, ui::EF_SHIFT_DOWN | ui::EF_PLATFORM_ACCELERATOR,
+           IDC_CYCLE_THIRD_PARTY_LLM_PROVIDER});
+      accelerators->push_back(
+          {ui::VKEY_A, ui::EF_ALT_DOWN, IDC_TOGGLE_BROWSEROS_AGENT});
+    }
+
     if (base::FeatureList::IsEnabled(features::kUIDebugTools)) {
       accelerators->insert(accelerators->begin(),
                            std::begin(kUIDebugAcceleratorMap),
