diff --git a/chrome/browser/global_keyboard_shortcuts_mac.mm b/chrome/browser/global_keyboard_shortcuts_mac.mm
index 56da23fd6d745f53d7d652b47168aa171451a4ea..dd4c8e00b1d0b4773db72dd8e6556303a26a0dca 100644
--- a/chrome/browser/global_keyboard_shortcuts_mac.mm
+++ b/chrome/browser/global_keyboard_shortcuts_mac.mm
@@ -148,6 +148,7 @@ const std::vector<KeyboardShortcutData>& GetShortcutsNotPresentInMainMenu() {
       {true,  true,  false, false, kVK_ANSI_L,            IDC_SHOW_THIRD_PARTY_LLM_SIDE_PANEL},
       {true,  true,  false, false, kVK_ANSI_Semicolon,   IDC_CYCLE_THIRD_PARTY_LLM_PROVIDER},
       {true,  true,  false, false, kVK_ANSI_U,            IDC_OPEN_CLASH_OF_GPTS},
+      {true,  true,  false, false, kVK_ANSI_E,            IDC_TOGGLE_AGENT_SPLIT},
       {true,  true,  false, false, kVK_ANSI_C,            IDC_DEV_TOOLS_INSPECT},
       {true,  false, false, true,  kVK_ANSI_C,            IDC_DEV_TOOLS_INSPECT},
       {true,  false, false, true,  kVK_DownArrow,         IDC_FOCUS_NEXT_PANE},
