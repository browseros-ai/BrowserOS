diff --git a/chrome/app/chrome_command_ids.h b/chrome/app/chrome_command_ids.h
index 2fe1c5d1ff982..aef3c11d6be07 100644
--- a/chrome/app/chrome_command_ids.h
+++ b/chrome/app/chrome_command_ids.h
@@ -309,6 +309,11 @@
 #define IDC_CONTENT_CONTEXT_INSPECTELEMENT_WITH_GEMINI 40300
 #define IDC_CONTENT_CONTEXT_INSPECTELEMENT_WITH_DEVTOOLS 40301
 #define IDC_REPORT_UNSAFE_SITE 40302
+// BrowserOS: custom command IDs
+#define IDC_SHOW_THIRD_PARTY_LLM_SIDE_PANEL  40303
+#define IDC_CYCLE_THIRD_PARTY_LLM_PROVIDER  40304
+#define IDC_OPEN_CLASH_OF_GPTS               40305
+#define IDC_TOGGLE_BROWSEROS_AGENT           40306
 
 // Spell-check
 // Insert any additional suggestions before _LAST; these have to be consecutive.
