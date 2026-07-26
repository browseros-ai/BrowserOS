diff --git a/chrome/browser/ui/toasts/api/toast_id.cc b/chrome/browser/ui/toasts/api/toast_id.cc
index 6a52e958fdcb5..8d2e1ae503c60 100644
--- a/chrome/browser/ui/toasts/api/toast_id.cc
+++ b/chrome/browser/ui/toasts/api/toast_id.cc
@@ -75,6 +75,8 @@ std::string_view GetToastName(ToastId toast_id) {
       return "MultistepFilterSuggestionRecent";
     case ToastId::kSkillSavedWithoutInvokeButton:
       return "SkillSavedWithoutInvokeButton";
+    case ToastId::kBrowserOSToast:
+      return "BrowserOSToast";
   }
 
   NOTREACHED();
