diff --git a/chrome/browser/ui/accelerator_table.cc b/chrome/browser/ui/accelerator_table.cc
index 171a1037969db..664d80420acb1 100644
--- a/chrome/browser/ui/accelerator_table.cc
+++ b/chrome/browser/ui/accelerator_table.cc
@@ -155,6 +155,10 @@ const AcceleratorMapping kAcceleratorMap[] = {
     {ui::VKEY_F11, ui::EF_NONE, IDC_FULLSCREEN},
     {ui::VKEY_M, ui::EF_SHIFT_DOWN | ui::EF_PLATFORM_ACCELERATOR,
      IDC_SHOW_AVATAR_MENU},
+    {ui::VKEY_K, ui::EF_ALT_DOWN, IDC_SHOW_THIRD_PARTY_LLM_SIDE_PANEL},
+    {ui::VKEY_L, ui::EF_ALT_DOWN, IDC_CYCLE_THIRD_PARTY_LLM_PROVIDER},
+    {ui::VKEY_U, ui::EF_SHIFT_DOWN | ui::EF_ALT_DOWN, IDC_OPEN_CLASH_OF_GPTS},
+    {ui::VKEY_E, ui::EF_ALT_DOWN, IDC_TOGGLE_BROWSEROS_AGENT},
 
 // Platform-specific key maps.
 #if BUILDFLAG(IS_LINUX) || BUILDFLAG(IS_CHROMEOS)
