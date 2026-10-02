diff --git a/base/threading/thread_restrictions.h b/base/threading/thread_restrictions.h
index db88f6ab7629f41a1c51945c570a82c35e9ef723..4dbd375f1d10a3f0caa0b7fc78c49d5fa2c7f91a 100644
--- a/base/threading/thread_restrictions.h
+++ b/base/threading/thread_restrictions.h
@@ -206,6 +206,9 @@ namespace scheduler {
 class NonMainThreadImpl;
 }
 }  // namespace blink
+namespace browseros {
+class BrowserOSServerManager;
+}  // namespace browseros
 namespace cc {
 class CategorizedWorkerPoolJob;
 class CategorizedWorkerPool;
@@ -622,6 +625,7 @@ class BASE_EXPORT ScopedAllowBlocking {
   friend class base::subtle::PlatformSharedMemoryRegion;
   friend class base::win::ScopedAllowBlockingForUserAccountControl;
   friend class blink::DiskDataAllocator;
+  friend class browseros::BrowserOSServerManager;
   friend class chromecast::CrashUtil;
   friend class content::BrowserProcessIOThread;
   friend class content::DWriteFontProxyImpl;
@@ -771,6 +775,7 @@ class BASE_EXPORT ScopedAllowBaseSyncPrimitives {
   friend class base::SimpleThread;
   friend class base::internal::GetAppOutputScopedAllowBaseSyncPrimitives;
   friend class blink::SourceStream;
+  friend class browseros::BrowserOSServerManager;
   friend class blink::VideoTrackRecorderImplContextProvider;
   friend class blink::WorkerThread;
   friend class blink::scheduler::NonMainThreadImpl;
