diff --git a/base/threading/thread_restrictions.h b/base/threading/thread_restrictions.h
index a1bb1a4602902..3f606d1b23986 100644
--- a/base/threading/thread_restrictions.h
+++ b/base/threading/thread_restrictions.h
@@ -205,6 +205,9 @@ namespace scheduler {
 class NonMainThreadImpl;
 }
 }  // namespace blink
+namespace browseros {
+class BrowserOSServerManager;
+}  // namespace browseros
 namespace cc {
 class CategorizedWorkerPoolJob;
 class CategorizedWorkerPool;
@@ -614,6 +617,7 @@ class BASE_EXPORT ScopedAllowBlocking {
   friend class base::subtle::PlatformSharedMemoryRegion;
   friend class base::win::ScopedAllowBlockingForUserAccountControl;
   friend class blink::DiskDataAllocator;
+  friend class browseros::BrowserOSServerManager;
   friend class chromecast::CrashUtil;
   friend class content::BrowserProcessIOThread;
   friend class content::DWriteFontProxyImpl;
@@ -763,6 +767,7 @@ class BASE_EXPORT ScopedAllowBaseSyncPrimitives {
   friend class base::SimpleThread;
   friend class base::internal::GetAppOutputScopedAllowBaseSyncPrimitives;
   friend class blink::SourceStream;
+  friend class browseros::BrowserOSServerManager;
   friend class blink::VideoTrackRecorderImplContextProvider;
   friend class blink::WorkerThread;
   friend class blink::scheduler::NonMainThreadImpl;
