diff --git a/chrome/browser/browseros/server/server_version_store.h b/chrome/browser/browseros/server/server_version_store.h
new file mode 100644
index 0000000000000000000000000000000000000000..be6ac747f34ecfe74c89b916d708cbe1f05687fa
--- /dev/null
+++ b/chrome/browser/browseros/server/server_version_store.h
@@ -0,0 +1,103 @@
+#ifndef CHROME_BROWSER_BROWSEROS_SERVER_SERVER_VERSION_STORE_H_
+#define CHROME_BROWSER_BROWSEROS_SERVER_SERVER_VERSION_STORE_H_
+
+#include <optional>
+
+#include "base/files/file_path.h"
+#include "base/functional/callback.h"
+#include "base/memory/weak_ptr.h"
+#include "base/task/sequenced_task_runner.h"
+#include "base/version.h"
+
+namespace browseros {
+
+struct ManagedServerDescriptor;
+
+// A launch selection keeps a probed version, executable, and resources
+// together. The manager copies it before launching; later downloads cannot
+// change the identity of an in-flight launch or an already-running process.
+struct ServerInstallation {
+  base::Version version;
+  base::FilePath executable;
+  base::FilePath resources;
+  bool downloaded = false;
+};
+
+// UI-sequence catalog of bundled and prepared server installations. File I/O
+// and bounded --version subprocesses run on one worker sequence. The legacy
+// current_version file identifies a READY installation, never a running
+// process.
+class ServerVersionStore {
+ public:
+  ServerVersionStore(const base::FilePath& execution_dir,
+                     ServerInstallation bundled,
+                     const ManagedServerDescriptor& descriptor);
+  ~ServerVersionStore();
+
+  ServerVersionStore(const ServerVersionStore&) = delete;
+  ServerVersionStore& operator=(const ServerVersionStore&) = delete;
+
+  void Initialize(base::OnceClosure callback);
+  // Drain local writes before the manager releases the profile lock. Otherwise
+  // a cancelled old session could overwrite the next session's ready marker.
+  void Shutdown(base::OnceClosure callback);
+  bool initialized() const { return initialized_; }
+  const ServerInstallation& GetBestAvailable() const;
+  const ServerInstallation& GetBundled() const { return bundled_; }
+
+  // Publishes a ready selection only after the expected binary version is
+  // verified and its marker is atomically persisted. Failure preserves any
+  // other ready installation, allowing a working server to remain selected.
+  void PrepareVersion(const base::Version& version,
+                      base::OnceCallback<void(bool)> callback);
+
+  // Removes an installation that can never run, such as one whose executable
+  // is missing. Call after its process has stopped, so Windows can remove its
+  // files and a live server never loses its resources. Clears only this
+  // version.
+  void RemoveInvalidVersion(const base::Version& version);
+
+  // A downloaded version that failed under supervision. GetBestAvailable()
+  // skips it for the rest of this session, but its files are kept: it may have
+  // migrated user data past what the bundled server can open, and then it is
+  // the only binary that can still serve. An invalid version readmits it.
+  void set_rejected_version(const base::Version& version) {
+    rejected_ = version;
+  }
+  const base::Version& rejected_version() const { return rejected_; }
+
+  // The ready download even if rejected. Supervision falls back to it when the
+  // bundled server is out of strikes too.
+  const std::optional<ServerInstallation>& fallback_download() const {
+    return ready_;
+  }
+
+  base::FilePath GetVersionsDir() const;
+  base::FilePath GetVersionDir(const base::Version& version) const;
+  base::FilePath GetPendingUpdateDir() const;
+
+  // Archive extraction and pruning use this same sequence so they cannot race
+  // marker publication or rejection of a previously selected installation.
+  scoped_refptr<base::SequencedTaskRunner> file_task_runner() const {
+    return task_runner_;
+  }
+
+ private:
+  ServerInstallation GetDownloadedInstallation(
+      const base::Version& version) const;
+
+  const base::FilePath execution_dir_;
+  const base::FilePath binary_name_;
+  ServerInstallation bundled_;
+  std::optional<ServerInstallation> ready_;
+  // Excluded from GetBestAvailable(). Invalid when nothing is rejected.
+  base::Version rejected_;
+  bool initialized_ = false;
+
+  scoped_refptr<base::SequencedTaskRunner> task_runner_;
+  base::WeakPtrFactory<ServerVersionStore> weak_factory_{this};
+};
+
+}  // namespace browseros
+
+#endif  // CHROME_BROWSER_BROWSEROS_SERVER_SERVER_VERSION_STORE_H_
