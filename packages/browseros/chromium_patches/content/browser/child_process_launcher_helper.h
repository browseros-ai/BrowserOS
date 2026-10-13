diff --git a/content/browser/child_process_launcher_helper.h b/content/browser/child_process_launcher_helper.h
index 407c06301325c886bb2489a1b67dc25d43bd9cf2..8376474e0606bef327516eca8770a021836408bd 100644
--- a/content/browser/child_process_launcher_helper.h
+++ b/content/browser/child_process_launcher_helper.h
@@ -5,8 +5,10 @@
 #ifndef CONTENT_BROWSER_CHILD_PROCESS_LAUNCHER_HELPER_H_
 #define CONTENT_BROWSER_CHILD_PROCESS_LAUNCHER_HELPER_H_
 
+#include <cstdint>
 #include <memory>
 #include <optional>
+#include <string>
 
 #include "base/memory/raw_ptr.h"
 #include "base/memory/read_only_shared_memory_region.h"
@@ -76,6 +78,65 @@ class PosixFileDescriptorInfo;
 
 namespace internal {
 
+#if BUILDFLAG(IS_MAC)
+// Authority for diagnostic receipts comes from an accepted RPH lifecycle
+// decision, never from a generic launcher destructor or normal-looking exit
+// code.
+enum class NormalRendererShutdownReason {
+  kAcceptedFastShutdown,
+  kUnusedSpareOverridden,
+  kUnusedSpareNotEnabled,
+  kUnusedSpareProcessLimit,
+  kUnusedSpareTimeout,
+  kUnusedSpareMemoryPressure,
+};
+
+enum class RendererShutdownDiagnosticInvalidation {
+  kGenericProcessDied,
+  kGenericTerminate,
+  kProcessNotRunning,
+  kIntentAlreadyRecorded,
+};
+
+// Numeric kernel identity only. Executable/argv ownership remains the external
+// observer's responsibility; these fields cannot establish a target/page
+// mapping.
+struct RendererProcessIdentityForDiagnostics {
+  uint32_t pid;
+  uint64_t start_s;
+  uint64_t start_us;
+  uint32_t uid;
+  uint32_t ppid;
+  uint32_t pgid;
+  uint32_t bsd_status;
+  uint32_t bsd_flags;
+  uint32_t bsd_xstatus;
+};
+
+// Keep failed reads distinguishable from real metadata. Clock values bracket
+// proc_pidinfo using explicit CLOCK_MONOTONIC, not overridable TimeTicks.
+struct RendererProcessIdentityReadForDiagnostics {
+  std::optional<base::ProcessId> requested_pid;
+  std::optional<int> read_result;
+  int expected_size;
+  std::optional<int64_t> read_start_ns;
+  std::optional<int64_t> read_end_ns;
+  std::optional<RendererProcessIdentityForDiagnostics> identity;
+};
+
+// Owned immutably by a single move-only Process across the UI/launcher handoff.
+// Only a matching fresh pre-signal read and the original call's successful
+// SIGTERM result can complete this intent; an intent log alone grants nothing.
+struct RendererShutdownIntentForDiagnostics {
+  std::string token;
+  NormalRendererShutdownReason reason;
+  ChildProcessId host_id;
+  RendererProcessIdentityReadForDiagnostics browser;
+  RendererProcessIdentityReadForDiagnostics child;
+  std::optional<int64_t> decision_ns;
+};
+#endif  // BUILDFLAG(IS_MAC)
+
 #if BUILDFLAG(IS_POSIX) || BUILDFLAG(IS_FUCHSIA)
 using FileMappedForLaunch = PosixFileDescriptorInfo;
 #else
@@ -109,6 +170,13 @@ class ChildProcessLauncherHelper
 
     base::Process process;
 
+#if BUILDFLAG(IS_MAC)
+    std::unique_ptr<const RendererShutdownIntentForDiagnostics>
+        renderer_shutdown_intent;
+    // Once invalidated or consumed, this incarnation cannot mint a new token.
+    bool renderer_shutdown_diagnostic_disqualified = false;
+#endif
+
 #if BUILDFLAG(USE_ZYGOTE)
     raw_ptr<ZygoteCommunication> zygote = nullptr;
 #endif  // BUILDFLAG(USE_ZYGOTE)
@@ -235,6 +303,17 @@ class ChildProcessLauncherHelper
   // Note that |exit_code| is not used on Android.
   static bool TerminateProcess(const base::Process& process, int exit_code);
 
+#if BUILDFLAG(IS_MAC)
+  static bool RendererShutdownDiagnosticsEnabled();
+  static void RecordNormalRendererShutdownIntentForDiagnostics(
+      Process& process,
+      ChildProcessId host_id,
+      NormalRendererShutdownReason reason);
+  static void InvalidateRendererShutdownIntentForDiagnostics(
+      Process& process,
+      RendererShutdownDiagnosticInvalidation cause);
+#endif
+
   // Terminates the process with the normal exit code and ensures it has been
   // stopped. By returning a normal exit code this ensures UMA won't treat this
   // as a crash.
