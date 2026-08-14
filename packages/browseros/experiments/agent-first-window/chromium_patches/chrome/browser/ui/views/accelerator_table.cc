diff --git a/chrome/browser/ui/views/accelerator_table.cc b/chrome/browser/ui/views/accelerator_table.cc
index 80af2177736e3cde2ccda0f80b1af21f34eddc3c..befed98df61891778829d50c228c124f6b12f985 100644
--- a/chrome/browser/ui/views/accelerator_table.cc
+++ b/chrome/browser/ui/views/accelerator_table.cc
@@ -155,6 +155,8 @@ const AcceleratorMapping kAcceleratorMap[] = {
      IDC_SHOW_THIRD_PARTY_LLM_SIDE_PANEL},
     {ui::VKEY_U, ui::EF_SHIFT_DOWN | ui::EF_PLATFORM_ACCELERATOR,
      IDC_OPEN_CLASH_OF_GPTS},
+    {ui::VKEY_E, ui::EF_SHIFT_DOWN | ui::EF_PLATFORM_ACCELERATOR,
+     IDC_TOGGLE_AGENT_SPLIT},
 
 // Platform-specific key maps.
 #if BUILDFLAG(IS_LINUX) || BUILDFLAG(IS_CHROMEOS)
