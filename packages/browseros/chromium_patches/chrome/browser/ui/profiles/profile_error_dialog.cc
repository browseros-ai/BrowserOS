diff --git a/chrome/browser/ui/profiles/profile_error_dialog.cc b/chrome/browser/ui/profiles/profile_error_dialog.cc
index 3a64be42331a3..66d8390670219 100644
--- a/chrome/browser/ui/profiles/profile_error_dialog.cc
+++ b/chrome/browser/ui/profiles/profile_error_dialog.cc
@@ -69,10 +69,11 @@ void ShowProfileErrorDialog(ProfileErrorType type,
       l10n_util::GetStringUTF16(IDS_PROFILE_ERROR_DIALOG_CHECKBOX),
       base::BindOnce(&OnProfileErrorDialogDismissed, diagnostics));
 #else   // BUILDFLAG(GOOGLE_CHROME_BRANDING)
-  chrome::ShowWarningMessageBoxAsync(
-      gfx::NativeWindow(),
-      l10n_util::GetStringUTF16(IDS_PROFILE_ERROR_DIALOG_TITLE),
-      l10n_util::GetStringUTF16(message_id));
+  // FIXME: nikhil: Handle this warning better
+  // chrome::ShowWarningMessageBox(
+  //     gfx::NativeWindow(),
+  //     l10n_util::GetStringUTF16(IDS_PROFILE_ERROR_DIALOG_TITLE),
+  //     l10n_util::GetStringUTF16(message_id));
 #endif  // BUILDFLAG(GOOGLE_CHROME_BRANDING)
 
 #endif  // BUILDFLAG(IS_ANDROID)
