diff --git a/chrome/app/chrome_crash_reporter_client_win.h b/chrome/app/chrome_crash_reporter_client_win.h
index 58c146d02e27b7d9075055e7e2f52af7e148352f..fb178cd1ff367f2d24d83cda52da71d863759ccd 100644
--- a/chrome/app/chrome_crash_reporter_client_win.h
+++ b/chrome/app/chrome_crash_reporter_client_win.h
@@ -50,6 +50,8 @@ class ChromeCrashReporterClient : public crash_reporter::CrashReporterClient {
   std::vector<base::ReadOnlySharedMemoryRegion>
   GetUserStreamSharedMemoryRegions() override;
 
+  std::string GetUploadUrl() override;
+
   std::wstring GetWerRuntimeExceptionModule() override;
 };
 
