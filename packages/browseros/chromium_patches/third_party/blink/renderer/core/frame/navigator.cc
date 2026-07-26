diff --git a/third_party/blink/renderer/core/frame/navigator.cc b/third_party/blink/renderer/core/frame/navigator.cc
index 583f05f8dd5fb..90a9a3a91b9a2 100644
--- a/third_party/blink/renderer/core/frame/navigator.cc
+++ b/third_party/blink/renderer/core/frame/navigator.cc
@@ -98,12 +98,7 @@ bool Navigator::cookieEnabled() const {
 }
 
 bool Navigator::webdriver() const {
-  if (RuntimeEnabledFeatures::AutomationControlledEnabled())
-    return true;
-
-  bool automation_enabled = false;
-  probe::ApplyAutomationOverride(GetExecutionContext(), automation_enabled);
-  return automation_enabled;
+  return false;
 }
 
 String Navigator::GetAcceptLanguages() {
