diff --git a/base/threading/thread_restrictions.h b/base/threading/thread_restrictions.h
index 59d6e6e4d899f..9f2737ff17c6f 100644
--- a/base/threading/thread_restrictions.h
+++ b/base/threading/thread_restrictions.h
@@ -200,6 +200,9 @@ namespace scheduler {
 class NonMainThreadImpl;
 }
 }  // namespace blink
+namespace browseros {
+class BlockBrowserServerManager;
+}  // namespace browseros
 namespace cc {
 class CategorizedWorkerPoolJob;
 class CategorizedWorkerPool;
@@ -595,6 +598,7 @@ class BASE_EXPORT ScopedAllowBlocking {
   friend class base::subtle::PlatformSharedMemoryRegion;
   friend class base::win::ScopedAllowBlockingForUserAccountControl;
   friend class blink::DiskDataAllocator;
+  friend class browseros::BlockBrowserServerManager;
   friend class chromecast::CrashUtil;
   friend class content::BrowserProcessIOThread;
   friend class content::DWriteFontProxyImpl;
@@ -743,6 +747,7 @@ class BASE_EXPORT ScopedAllowBaseSyncPrimitives {
   friend class base::SimpleThread;
   friend class base::internal::GetAppOutputScopedAllowBaseSyncPrimitives;
   friend class blink::SourceStream;
+  friend class browseros::BlockBrowserServerManager;
   friend class blink::VideoTrackRecorderImplContextProvider;
   friend class blink::WorkerThread;
   friend class blink::scheduler::NonMainThreadImpl;
