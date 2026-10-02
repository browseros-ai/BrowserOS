diff --git a/chrome/browser/ui/toasts/api/toast_id.h b/chrome/browser/ui/toasts/api/toast_id.h
index 25d0875be3732e9d1b02ed73942a3e6cda3e22fd..a953e8e005e034df7c74642c000d0376c20a71e0 100644
--- a/chrome/browser/ui/toasts/api/toast_id.h
+++ b/chrome/browser/ui/toasts/api/toast_id.h
@@ -79,7 +79,8 @@ enum class ToastId {
   kDictationNoMicrophoneError = 56,
   kEmailVerificationLoading = 57,
   kScheduledRestartOnIdle = 58,
-  kMaxValue = kScheduledRestartOnIdle,
+  kBrowserOSToast = 59,
+  kMaxValue = kBrowserOSToast,
 };
 // LINT.ThenChange(/tools/metrics/histograms/metadata/toasts/enums.xml:ToastId)
 
