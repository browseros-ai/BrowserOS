diff --git a/content/browser/child_process_launcher_helper_mac.cc b/content/browser/child_process_launcher_helper_mac.cc
index 72fb33d264e8fb0872be8cace5a4ca19ada53372..ab38298b1f8bc0aac2e666b7a3e6e19aa28066de 100644
--- a/content/browser/child_process_launcher_helper_mac.cc
+++ b/content/browser/child_process_launcher_helper_mac.cc
@@ -4,15 +4,29 @@
 
 #include "content/browser/child_process_launcher_helper.h"
 
+#include <libproc.h>
+#include <sys/proc.h>
+#include <sys/proc_info.h>
+#include <time.h>
+#include <unistd.h>
+
+#include <limits>
+#include <locale>
+#include <sstream>
+#include <string_view>
+#include <vector>
+
 #include "base/apple/mach_port_rendezvous.h"
 #include "base/command_line.h"
 #include "base/containers/flat_map.h"
+#include "base/logging.h"
 #include "base/no_destructor.h"
 #include "base/path_service.h"
 #include "base/posix/global_descriptors.h"
 #include "base/strings/stringprintf.h"
 #include "base/synchronization/lock.h"
 #include "base/thread_annotations.h"
+#include "base/unguessable_token.h"
 #include "content/browser/child_process_launcher.h"
 #include "content/browser/child_process_launcher_helper_posix.h"
 #include "content/browser/child_process_task_port_provider_mac.h"
@@ -37,6 +51,213 @@ namespace internal {
 
 namespace {
 
+using Identity = RendererProcessIdentityForDiagnostics;
+using IdentityRead = RendererProcessIdentityReadForDiagnostics;
+using Intent = RendererShutdownIntentForDiagnostics;
+using Problems = std::vector<std::string_view>;
+
+// Use the exact OS clock selected by the external observer. Mac TimeTicks and
+// Python's implicit monotonic implementation are not this receipt's contract.
+std::optional<int64_t> ReadDiagnosticMonotonicNs() {
+  timespec now;
+  if (clock_gettime(CLOCK_MONOTONIC, &now) != 0 || now.tv_sec < 0 ||
+      now.tv_nsec < 0 || now.tv_nsec >= 1000000000 ||
+      now.tv_sec >
+          (std::numeric_limits<int64_t>::max() - now.tv_nsec) / 1000000000) {
+    return std::nullopt;
+  }
+  return now.tv_sec * int64_t{1000000000} + now.tv_nsec;
+}
+
+IdentityRead ReadIdentityForDiagnostics(base::ProcessId pid) {
+  IdentityRead read{.expected_size = sizeof(proc_bsdinfo)};
+  if (pid <= 0) {
+    return read;
+  }
+  read.requested_pid = pid;
+  proc_bsdinfo info{};
+  read.read_start_ns = ReadDiagnosticMonotonicNs();
+  read.read_result =
+      proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, sizeof(info));
+  read.read_end_ns = ReadDiagnosticMonotonicNs();
+  // A partial or failed read is evidence of failure, not a zero-filled
+  // identity.
+  if (*read.read_result == read.expected_size &&
+      info.pbi_pid == static_cast<uint32_t>(pid) &&
+      info.pbi_pid > 0 && info.pbi_pgid > 0 && info.pbi_start_tvsec > 0 &&
+      info.pbi_start_tvusec < 1000000) {
+    read.identity = Identity{.pid = info.pbi_pid,
+                             .start_s = info.pbi_start_tvsec,
+                             .start_us = info.pbi_start_tvusec,
+                             .uid = info.pbi_uid,
+                             .ppid = info.pbi_ppid,
+                             .pgid = info.pbi_pgid,
+                             .bsd_status = info.pbi_status,
+                             .bsd_flags = info.pbi_flags,
+                             .bsd_xstatus = info.pbi_xstatus};
+  }
+  return read;
+}
+
+bool UsableRead(const IdentityRead& read) {
+  return read.identity && read.requested_pid && read.read_result &&
+         *read.read_result == read.expected_size && read.read_start_ns &&
+         read.read_end_ns && *read.read_start_ns <= *read.read_end_ns;
+}
+
+bool Nonterminal(const Identity& identity) {
+  // PROC_FLAG_EXEC means "has called exec" and is intentionally retained.
+  // These are proc_bsdinfo flags, not the unrelated extern_proc P_* flags.
+  return identity.bsd_status >= SIDL && identity.bsd_status <= SSTOP &&
+         !(identity.bsd_flags & PROC_FLAG_INEXIT) && identity.bsd_xstatus == 0;
+}
+
+bool SameIncarnation(const Identity& first, const Identity& current) {
+  return first.pid == current.pid && first.start_s == current.start_s &&
+         first.start_us == current.start_us && first.uid == current.uid &&
+         first.ppid == current.ppid && first.pgid == current.pgid;
+}
+
+std::string_view ReasonName(NormalRendererShutdownReason reason) {
+  switch (reason) {
+    case NormalRendererShutdownReason::kAcceptedFastShutdown:
+      return "accepted_fast_shutdown";
+    case NormalRendererShutdownReason::kUnusedSpareOverridden:
+      return "unused_spare_overridden";
+    case NormalRendererShutdownReason::kUnusedSpareNotEnabled:
+      return "unused_spare_not_enabled";
+    case NormalRendererShutdownReason::kUnusedSpareProcessLimit:
+      return "unused_spare_process_limit";
+    case NormalRendererShutdownReason::kUnusedSpareTimeout:
+      return "unused_spare_timeout";
+    case NormalRendererShutdownReason::kUnusedSpareMemoryPressure:
+      return "unused_spare_memory_pressure";
+  }
+  return "unknown";  // A malformed enum must fail the closed V1 grammar.
+}
+
+std::string_view InvalidationName(
+    RendererShutdownDiagnosticInvalidation cause) {
+  switch (cause) {
+    case RendererShutdownDiagnosticInvalidation::kGenericProcessDied:
+      return "generic_process_died";
+    case RendererShutdownDiagnosticInvalidation::kGenericTerminate:
+      return "generic_terminate";
+    case RendererShutdownDiagnosticInvalidation::kProcessNotRunning:
+      return "process_not_running";
+    case RendererShutdownDiagnosticInvalidation::kIntentAlreadyRecorded:
+      return "intent_already_recorded";
+  }
+  return "diagnostic_disqualified";
+}
+
+template <typename T>
+void WriteNumber(std::ostream& out, const std::optional<T>& value) {
+  if (value) {
+    out << *value;
+  } else {
+    out << "null";
+  }
+}
+
+void WriteRead(std::ostream& out, const IdentityRead& read) {
+  out << "{\"requested_pid\":";
+  WriteNumber(out, read.requested_pid);
+  out << ",\"read_result\":";
+  WriteNumber(out, read.read_result);
+  out << ",\"expected_size\":" << read.expected_size << ",\"read_start_ns\":";
+  WriteNumber(out, read.read_start_ns);
+  out << ",\"read_end_ns\":";
+  WriteNumber(out, read.read_end_ns);
+  out << ",\"identity\":";
+  if (read.identity) {
+    const auto& id = *read.identity;
+    out << "{\"pid\":" << id.pid << ",\"start_s\":" << id.start_s
+        << ",\"start_us\":" << id.start_us << ",\"uid\":" << id.uid
+        << ",\"ppid\":" << id.ppid << ",\"pgid\":" << id.pgid
+        << ",\"bsd_status\":" << id.bsd_status
+        << ",\"bsd_flags\":" << id.bsd_flags
+        << ",\"bsd_xstatus\":" << id.bsd_xstatus << "}";
+  } else {
+    out << "null";
+  }
+  out << "}";
+}
+
+struct SignalDiagnosticResult {
+  std::optional<int64_t> request_ns;
+  std::optional<int64_t> completed_ns;
+  bool terminate_result;
+};
+
+void EmitRendererShutdownDiagnostic(std::string_view phase,
+                                    const Intent& intent,
+                                    const IdentityRead* pre_signal,
+                                    const SignalDiagnosticResult* signal,
+                                    std::optional<int64_t> event_ns,
+                                    bool eligible,
+                                    const Problems& problems) {
+  if (!ChildProcessLauncherHelper::RendererShutdownDiagnosticsEnabled()) {
+    return;
+  }
+  // V1 contains only integer/null values and closed ASCII strings/hex tokens.
+  // Stream integers directly: base::Value's double representation would lose
+  // precision for nanosecond clocks. No caller-controlled strings are
+  // serialized.
+  std::ostringstream out;
+  out.imbue(std::locale::classic());
+  out << "{\"schema_version\":1,\"contract\":\"renderer-normal-sigterm-v1\","
+         "\"source\":\"content/browser/child_process_launcher_helper_mac.cc\","
+         "\"phase\":\""
+      << phase << "\",\"token\":";
+  if (intent.token.empty()) {
+    out << "null";
+  } else {
+    out << '"' << intent.token << '"';
+  }
+  out << ",\"reason\":\"" << ReasonName(intent.reason)
+      << "\",\"host_id\":" << intent.host_id.value()
+      << ",\"clock\":{\"name\":\"CLOCK_MONOTONIC\",\"id\":"
+      << static_cast<int>(CLOCK_MONOTONIC) << ",\"unit\":\"ns\"},\"browser\":";
+  WriteRead(out, intent.browser);
+  out << ",\"child\":";
+  WriteRead(out, intent.child);
+  out << ",\"decision_ns\":";
+  WriteNumber(out, intent.decision_ns);
+  out << ",\"pre_signal\":";
+  if (pre_signal) {
+    WriteRead(out, *pre_signal);
+  } else {
+    out << "null";
+  }
+  out << ",\"signal\":";
+  if (signal) {
+    out << "{\"number\":15,\"request_ns\":";
+    WriteNumber(out, signal->request_ns);
+    out << ",\"completed_ns\":";
+    WriteNumber(out, signal->completed_ns);
+    out << ",\"terminate_result\":"
+        << (signal->terminate_result ? "true" : "false")
+        << ",\"errno\":null,\"errno_status\":\"unavailable\"}";
+  } else {
+    out << "null";
+  }
+  out << ",\"event_ns\":";
+  WriteNumber(out, event_ns);
+  out << ",\"eligible\":" << (eligible ? "true" : "false") << ",\"problems\":[";
+  for (size_t i = 0; i < problems.size(); ++i) {
+    if (i) {
+      out << ',';
+    }
+    out << '"' << problems[i] << '"';
+  }
+  out << "]}";
+  // Pin this single emitter's file/line with the reviewed binary receipt. The
+  // consumer also checks its real log header/browser PID, not just payload
+  // text.
+  VLOG(1) << "NATIVE_RENDERER_TERMINATION_DIAGNOSTIC_V1 " << out.str();
+}
+
 // Class that holds a map of SandboxTypes to serialized policy strings. Only
 // certain sandbox types can be cached, depending on the nature of the
 // runtime parameters that are bound into the profile.
@@ -215,6 +436,87 @@ ChildProcessTerminationInfo ChildProcessLauncherHelper::GetTerminationInfo(
   return info;
 }
 
+// static
+bool ChildProcessLauncherHelper::RendererShutdownDiagnosticsEnabled() {
+  return VLOG_IS_ON(1);
+}
+
+// static
+void ChildProcessLauncherHelper::
+    RecordNormalRendererShutdownIntentForDiagnostics(
+        Process& process,
+        ChildProcessId host_id,
+        NormalRendererShutdownReason reason) {
+  if (!RendererShutdownDiagnosticsEnabled()) {
+    return;
+  }
+  Problems problems;
+  if (process.renderer_shutdown_intent) {
+    InvalidateRendererShutdownIntentForDiagnostics(
+        process,
+        RendererShutdownDiagnosticInvalidation::kIntentAlreadyRecorded);
+    problems.push_back("intent_already_recorded");
+  }
+  if (process.renderer_shutdown_diagnostic_disqualified) {
+    problems.push_back("diagnostic_disqualified");
+  }
+  Intent intent{.reason = reason,
+                .host_id = host_id,
+                .browser = ReadIdentityForDiagnostics(getpid()),
+                .child = ReadIdentityForDiagnostics(
+                    process.process.IsValid() ? process.process.Pid() : 0),
+                .decision_ns = ReadDiagnosticMonotonicNs()};
+  if (!UsableRead(intent.browser)) {
+    problems.push_back("browser_metadata_unusable");
+  } else if (!Nonterminal(*intent.browser.identity)) {
+    problems.push_back("browser_not_nonterminal");
+  }
+  if (!UsableRead(intent.child)) {
+    problems.push_back("child_metadata_unusable");
+  } else if (!Nonterminal(*intent.child.identity)) {
+    problems.push_back("child_not_nonterminal");
+  }
+  if (intent.browser.identity && intent.child.identity &&
+      (intent.child.identity->ppid != intent.browser.identity->pid ||
+       intent.child.identity->uid != intent.browser.identity->uid ||
+       intent.child.identity->pgid != intent.browser.identity->pgid)) {
+    problems.push_back("ownership_mismatch");
+  }
+  if (!intent.decision_ns) {
+    problems.push_back("decision_clock_unavailable");
+  } else if (UsableRead(intent.browser) && UsableRead(intent.child) &&
+             (*intent.browser.read_end_ns > *intent.child.read_start_ns ||
+              *intent.child.read_end_ns > *intent.decision_ns)) {
+    problems.push_back("clock_order_invalid");
+  }
+  if (!problems.empty()) {
+    process.renderer_shutdown_diagnostic_disqualified = true;
+    EmitRendererShutdownDiagnostic("unusable_intent", intent, nullptr, nullptr,
+                                   intent.decision_ns, false, problems);
+    return;
+  }
+  intent.token = base::UnguessableToken::Create().ToString();
+  process.renderer_shutdown_intent = std::make_unique<const Intent>(intent);
+  EmitRendererShutdownDiagnostic("intent", intent, nullptr, nullptr,
+                                 intent.decision_ns, true, {});
+}
+
+// static
+void ChildProcessLauncherHelper::InvalidateRendererShutdownIntentForDiagnostics(
+    Process& process,
+    RendererShutdownDiagnosticInvalidation cause) {
+  if (!RendererShutdownDiagnosticsEnabled() &&
+      !process.renderer_shutdown_intent) {
+    return;
+  }
+  process.renderer_shutdown_diagnostic_disqualified = true;
+  if (auto intent = std::move(process.renderer_shutdown_intent)) {
+    EmitRendererShutdownDiagnostic("invalidated", *intent, nullptr, nullptr,
+                                   ReadDiagnosticMonotonicNs(), false,
+                                   {InvalidationName(cause)});
+  }
+}
+
 // static
 bool ChildProcessLauncherHelper::TerminateProcess(const base::Process& process,
                                                   int exit_code) {
@@ -229,7 +531,60 @@ void ChildProcessLauncherHelper::ForceNormalProcessTerminationSync(
   CHECK(CurrentlyOnProcessLauncherTaskRunner(), base::NotFatalUntil::M159);
   // Client has gone away, so just kill the process.  Using exit code 0 means
   // that UMA won't treat this as a crash.
-  process.process.Terminate(RESULT_CODE_NORMAL_EXIT, false);
+  // The immutable intent moves with Process through the existing PostTask. A
+  // tokenless generic destructor emits no V1 line and confers no normal
+  // authority.
+  auto intent = std::move(process.renderer_shutdown_intent);
+  const bool record = intent && RendererShutdownDiagnosticsEnabled();
+  std::optional<IdentityRead> pre_signal;
+  SignalDiagnosticResult signal{};
+  if (record) {
+    process.renderer_shutdown_diagnostic_disqualified = true;
+    pre_signal = ReadIdentityForDiagnostics(process.process.Pid());
+    signal.request_ns = ReadDiagnosticMonotonicNs();
+  }
+  // For precisely NORMAL_EXIT,false, the existing POSIX implementation returns
+  // true iff its SIGTERM kill succeeds; it cannot wait or escalate. Keep this
+  // original call once, even when the diagnostic evidence is unusable.
+  const bool terminated =
+      process.process.Terminate(RESULT_CODE_NORMAL_EXIT, false);
+  if (record) {
+    signal.completed_ns = ReadDiagnosticMonotonicNs();
+    signal.terminate_result = terminated;
+    Problems problems;
+    if (!UsableRead(*pre_signal)) {
+      problems.push_back("pre_signal_metadata_unusable");
+    } else {
+      if (!Nonterminal(*pre_signal->identity)) {
+        problems.push_back("pre_signal_not_nonterminal");
+      }
+      if (!SameIncarnation(*intent->child.identity, *pre_signal->identity)) {
+        problems.push_back("pre_signal_identity_mismatch");
+      }
+    }
+    if (!signal.request_ns) {
+      problems.push_back("signal_request_clock_unavailable");
+    }
+    if (!signal.completed_ns) {
+      problems.push_back("signal_completed_clock_unavailable");
+    }
+    if (UsableRead(*pre_signal) && signal.request_ns && signal.completed_ns &&
+        (*intent->decision_ns > *pre_signal->read_start_ns ||
+         *pre_signal->read_end_ns > *signal.request_ns ||
+         *signal.request_ns > *signal.completed_ns)) {
+      problems.push_back("clock_order_invalid");
+    }
+    if (!terminated) {
+      problems.push_back("terminate_failed");
+    }
+    // Completion is captured immediately after the real result, before log IO
+    // and EnsureProcessTerminated. Errno across the existing logger is
+    // explicitly unavailable. The observer requires completion BEFORE its first
+    // read START.
+    EmitRendererShutdownDiagnostic("signal_result", *intent, &*pre_signal,
+                                   &signal, signal.completed_ns,
+                                   problems.empty(), problems);
+  }
   base::EnsureProcessTerminated(std::move(process.process));
 }
 
