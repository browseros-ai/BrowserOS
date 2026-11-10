//! Neo: Thin N-API bindings for the hosted SDK door. JavaScript stays in Node; all browser
//! behavior lives in neo-runtime. Deferred promises cross back to the JS thread without
//! napi's tokio_rt feature, which would enter Tokio around synchronous getters.

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use napi::bindgen_prelude::ToNapiValue;
use napi::{Env, Error, JsObject, Result};
use napi_derive::napi;
use neo_runtime::{HostedRun, RunError};
use serde_json::json;
use tokio::runtime::{Builder, Runtime as TokioRuntime};

// This process owns one executor. Node's main thread is never entered into it by a getter.
static EXECUTOR: OnceLock<std::result::Result<TokioRuntime, String>> = OnceLock::new();

fn executor() -> Result<&'static TokioRuntime> {
    EXECUTOR
        .get_or_init(|| {
            Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|message| Error::from_reason(message.clone()))
}

fn promise<T, F>(env: Env, future: F) -> Result<JsObject>
where
    T: ToNapiValue + Send + 'static,
    F: Future<Output = Result<T>> + Send + 'static,
{
    let executor = executor()?;
    let (deferred, promise) = env.create_deferred()?;
    executor.spawn(async move {
        let result = future.await;
        // N-API invokes this closure on Node's thread, including object construction.
        deferred.resolve(move |_| result);
    });
    Ok(promise)
}

fn native_error(error: RunError) -> Error {
    // Preserve the named error in the rejection text; the SDK protocol carries method errors
    // separately, with their full names and call logs, exactly as it does in QuickJS.
    Error::from_reason(format!("{}: {}", error.name, error.message))
}

#[napi]
pub struct Runtime {
    runtime: neo_runtime::Runtime,
    cdp: CdpClient,
}

#[napi]
pub fn connect(env: Env, cdp_port: u16) -> Result<JsObject> {
    promise(env, async move {
        let cdp = CdpClient::connect(ConnectOptions {
            reconnect_policy: ReconnectPolicy::KeepTrying,
            ..ConnectOptions::new(cdp_port)
        })
        .await
        .map_err(|error| Error::from_reason(error.to_string()))?;
        match neo_runtime::Runtime::connect(cdp.clone()).await {
            Ok(runtime) => Ok(Runtime { runtime, cdp }),
            Err(error) => {
                cdp.disconnect().await;
                Err(Error::from_reason(error.to_string()))
            }
        }
    })
}

#[napi(object)]
pub struct OpenRunOptions {
    pub timeout_ms: u32,
    pub artifacts_dir: Option<String>,
}

#[napi]
impl Runtime {
    #[napi]
    pub fn open_run(&self, env: Env, options: OpenRunOptions) -> Result<JsObject> {
        let runtime = self.runtime.clone();
        promise(env, async move {
            let run = runtime
                .open_hosted_run(
                    Duration::from_millis(u64::from(options.timeout_ms)),
                    options.artifacts_dir.map(PathBuf::from),
                )
                .await
                .map_err(native_error)?;
            Ok(Run { run: Arc::new(run) })
        })
    }

    #[napi]
    pub fn close(&self, env: Env) -> Result<JsObject> {
        let runtime = self.runtime.clone();
        let cdp = self.cdp.clone();
        promise(env, async move {
            runtime.drain().await;
            cdp.disconnect().await;
            Ok(())
        })
    }
}

#[napi]
pub struct Run {
    run: Arc<HostedRun>,
}

#[napi]
impl Run {
    #[napi(getter)]
    pub fn init(&self) -> String {
        self.run.init().to_string()
    }

    #[napi]
    pub fn get(&self, request: String) -> Result<String> {
        self.run.get(&request).map_err(native_error)
    }

    #[napi]
    pub fn drain(&self, until: i64) -> String {
        self.run.drain(until)
    }

    #[napi]
    pub fn call_with_events(&self, env: Env, message: String) -> Result<JsObject> {
        // Start now, before the next JS call can arrive; only the reply waits on the executor.
        let reply = self.run.call_with_events(&message);
        promise(env, async move {
            let (reply, events) = reply.await.map_err(native_error)?;
            Ok(json!({ "reply": reply, "events": events }).to_string())
        })
    }

    #[napi]
    pub fn next_events(&self, env: Env) -> Result<JsObject> {
        let run = Arc::clone(&self.run);
        promise(
            env,
            async move { run.next_events().await.map_err(native_error) },
        )
    }

    #[napi]
    pub fn abort(&self, id: i64, reason: String) {
        if let Ok(id) = u64::try_from(id) {
            self.run.abort(id, reason);
        }
    }

    #[napi]
    pub fn subscribe(&self, request: String) -> Result<()> {
        self.run.subscribe(&request).map_err(native_error)
    }

    #[napi]
    pub fn log(&self, line: String) {
        self.run.log(line);
    }

    #[napi]
    pub fn finish(&self, env: Env) -> Result<JsObject> {
        let run = Arc::clone(&self.run);
        promise(env, async move {
            let outcome = run.finish().await.map_err(native_error)?;
            serde_json::to_string(&outcome).map_err(|error| Error::from_reason(error.to_string()))
        })
    }
}
