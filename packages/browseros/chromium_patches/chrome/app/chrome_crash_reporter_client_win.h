diff --git a/chrome/app/chrome_crash_reporter_client_win.h b/chrome/app/chrome_crash_reporter_client_win.h
index e2f6c030c3638..f55bb5ca3604f 100644
--- a/chrome/app/chrome_crash_reporter_client_win.h
+++ b/chrome/app/chrome_crash_reporter_client_win.h
@@ -47,6 +47,8 @@ class ChromeCrashReporterClient : public crash_reporter::CrashReporterClient {
 
   bool EnableBreakpadForProcess(const std::string& process_type) override;
 
+  std::string GetUploadUrl() override;
+
   std::wstring GetWerRuntimeExceptionModule() override;
 };
 
