diff --git a/extensions/browser/process_manager.h b/extensions/browser/process_manager.h
index 4b3622c6bf83e..e6a892569bec5 100644
--- a/extensions/browser/process_manager.h
+++ b/extensions/browser/process_manager.h
@@ -427,6 +427,11 @@ class ProcessManager : public KeyedService,
   // A map of the active service worker keepalives.
   ServiceWorkerKeepaliveDataMap service_worker_keepalives_;
 
+  // BrowserOS: Maps WorkerId to keepalive UUID for BrowserOS extensions that
+  // should never be terminated. These permanent keepalives prevent the service
+  // worker from being killed due to inactivity.
+  std::map<WorkerId, base::Uuid> browseros_permanent_keepalives_;
+
   // Must be last member, see doc on WeakPtrFactory.
   base::WeakPtrFactory<ProcessManager> weak_ptr_factory_{this};
 };
