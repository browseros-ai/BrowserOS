diff --git a/extensions/browser/process_manager.h b/extensions/browser/process_manager.h
index e467d7d6245c3..ba83e0a868ac1 100644
--- a/extensions/browser/process_manager.h
+++ b/extensions/browser/process_manager.h
@@ -439,6 +439,11 @@ class ProcessManager : public KeyedService,
   // A map of the active service worker keepalives.
   ServiceWorkerKeepaliveDataMap service_worker_keepalives_;
 
+  // BlockBrowser: Maps WorkerId to keepalive UUID for BlockBrowser extensions that
+  // should never be terminated. These permanent keepalives prevent the service
+  // worker from being killed due to inactivity.
+  std::map<WorkerId, base::Uuid> blockbrowser_permanent_keepalives_;
+
   // Must be last member, see doc on WeakPtrFactory.
   base::WeakPtrFactory<ProcessManager> weak_ptr_factory_{this};
 };
