diff --git a/chrome/app/chrome_crash_reporter_client.h b/chrome/app/chrome_crash_reporter_client.h
index e537e1864e42f..13af42b341b59 100644
--- a/chrome/app/chrome_crash_reporter_client.h
+++ b/chrome/app/chrome_crash_reporter_client.h
@@ -62,6 +62,8 @@ class ChromeCrashReporterClient : public crash_reporter::CrashReporterClient {
 
   bool EnableBreakpadForProcess(const std::string& process_type) override;
 
+  std::string GetUploadUrl() override;
+
  private:
   friend class base::NoDestructor<ChromeCrashReporterClient>;
 
