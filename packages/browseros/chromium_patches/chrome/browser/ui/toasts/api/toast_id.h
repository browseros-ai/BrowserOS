diff --git a/chrome/browser/ui/toasts/api/toast_id.h b/chrome/browser/ui/toasts/api/toast_id.h
index 5f3c3ff3443c8..32a88e78f3024 100644
--- a/chrome/browser/ui/toasts/api/toast_id.h
+++ b/chrome/browser/ui/toasts/api/toast_id.h
@@ -54,7 +54,8 @@ enum class ToastId {
   kMultistepFilterSuggestion = 31,
   kMultistepFilterSuggestionRecent = 32,
   kSkillSavedWithoutInvokeButton = 33,
-  kMaxValue = kSkillSavedWithoutInvokeButton,
+  kBrowserOSToast = 34,
+  kMaxValue = kBrowserOSToast,
 };
 // LINT.ThenChange(/tools/metrics/histograms/metadata/toasts/enums.xml:ToastId)
 
