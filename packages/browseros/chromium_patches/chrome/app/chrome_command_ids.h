diff --git a/chrome/app/chrome_command_ids.h b/chrome/app/chrome_command_ids.h
index 8c34db5f63d03..7ec83d4866c0f 100644
--- a/chrome/app/chrome_command_ids.h
+++ b/chrome/app/chrome_command_ids.h
@@ -306,6 +306,10 @@
 #define IDC_CONTENT_CONTEXT_INSPECTELEMENT_WITH_DEVTOOLS 40301
 #define IDC_REPORT_UNSAFE_SITE 40302
 #define IDC_SHOW_READING_MODE_KEYBOARD 40303
+// BrowserOS: custom command IDs
+#define IDC_SHOW_THIRD_PARTY_LLM_SIDE_PANEL  40304
+#define IDC_CYCLE_THIRD_PARTY_LLM_PROVIDER  40305
+#define IDC_TOGGLE_BROWSEROS_AGENT           40307
 
 // Spell-check
 // Insert any additional suggestions before _LAST; these have to be consecutive.
