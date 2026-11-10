//! Neo: Thin N-API bindings for the hosted SDK door. JavaScript stays in Node; all browser
//! behavior lives in neo-runtime. Only subscription and dispatch enter Tokio on Node's thread.

mod promise;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use napi::{Env, Error, JsObject, Result};
use napi_derive::napi;
use neo_runtime::HostedRun;
use serde_json::json;
use tokio::runtime::{Builder, Runtime as TokioRuntime};

use crate::promise::{host_error, native_error, promise};

/// Neo: One process-owned executor; it never enters Node's synchronous getter thread.
static EXECUTOR: OnceLock<std::result::Result<TokioRuntime, String>> = OnceLock::new();
/// Neo: A host run spans a whole worker; individual SDK calls keep their own timeouts.
const DEFAULT_TIMEOUT_MS: u32 = 24 * 60 * 60 * 1000;

/// Neo: Tokio's threads do not retain Node's event loop; pending promises retain only their work.
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

/// Neo: Owns one browser connection and its hosted runs.
#[napi]
pub struct Runtime {
    runtime: neo_runtime::Runtime,
    cdp: CdpClient,
}

/// Neo: Connects to the host's private browser; failure disconnects the partially opened client.
#[napi]
pub fn connect(env: Env, cdp_port: u16) -> Result<JsObject> {
    promise(env, async move {
        let cdp = CdpClient::connect(ConnectOptions {
            reconnect_policy: ReconnectPolicy::KeepTrying,
            ..ConnectOptions::new(cdp_port)
        })
        .await
        .map_err(host_error)?;
        match neo_runtime::Runtime::connect(cdp.clone()).await {
            Ok(runtime) => Ok(Runtime { runtime, cdp }),
            Err(error) => {
                cdp.disconnect().await;
                Err(host_error(error))
            }
        }
    })
}

/// Neo: The deadline is a host option, independent of any individual Playwright test timeout.
#[napi(object)]
pub struct OpenRunOptions {
    pub timeout_ms: Option<u32>,
    pub artifacts_dir: Option<String>,
}

/// Neo: Admission closes synchronously; native cleanup completes before CDP disconnects.
#[napi]
impl Runtime {
    #[napi]
    pub fn open_run(&self, env: Env, options: Option<OpenRunOptions>) -> Result<JsObject> {
        let runtime = self.runtime.clone();
        let options = options.unwrap_or(OpenRunOptions {
            timeout_ms: None,
            artifacts_dir: None,
        });
        promise(env, async move {
            let run = runtime
                .open_hosted_run(
                    Duration::from_millis(u64::from(
                        options.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
                    )),
                    options.artifacts_dir.map(PathBuf::from),
                )
                .await?;
            Ok(Run { run: Arc::new(run) })
        })
    }

    #[napi]
    pub fn close(&self, env: Env) -> Result<JsObject> {
        self.runtime.cancel_hosted_runs();
        let runtime = self.runtime.clone();
        let cdp = self.cdp.clone();
        promise(env, async move {
            runtime.shutdown_hosted_runs().await;
            cdp.disconnect().await;
            Ok(())
        })
    }
}

/// Neo: GC or environment unload must cancel runs even without an explicit close.
impl Drop for Runtime {
    fn drop(&mut self) {
        self.runtime.cancel_hosted_runs();
        let runtime = self.runtime.clone();
        let cdp = self.cdp.clone();
        if let Ok(executor) = executor() {
            executor.spawn(async move {
                runtime.shutdown_hosted_runs().await;
                cdp.disconnect().await;
            });
        }
    }
}

/// Neo: The JS owner explicitly cancels on finalization; promises never prolong its ownership.
#[napi]
pub struct Run {
    run: Arc<HostedRun>,
}

/// Neo: Every synchronous error and asynchronous rejection carries Rust's exact name and message.
#[napi]
impl Run {
    #[napi(getter)]
    pub fn init(&self) -> String {
        self.run.init().to_string()
    }

    #[napi]
    pub fn get(&self, env: Env, request: String) -> Result<String> {
        self.run
            .get(&request)
            .map_err(|error| native_error(env, error))
    }

    #[napi]
    pub fn drain(&self, env: Env, until: i64) -> Result<String> {
        self.run
            .drain(until)
            .map_err(|error| native_error(env, error))
    }

    #[napi]
    pub fn call_with_events(&self, env: Env, message: String) -> Result<JsObject> {
        let reply = self.run.call_with_events(&message);
        promise(env, async move {
            let (reply, events) = reply.await?;
            Ok(json!({ "reply": reply, "events": events }).to_string())
        })
    }

    #[napi]
    pub fn next_events(&self, env: Env) -> Result<JsObject> {
        promise(env, self.run.next_events())
    }

    #[napi]
    pub fn abort(&self, id: i64, reason: String) {
        if let Ok(id) = u64::try_from(id) {
            self.run.abort(id, reason);
        }
    }

    #[napi]
    pub fn subscribe(&self, env: Env, request: String) -> Result<()> {
        self.run
            .subscribe(&request)
            .map_err(|error| native_error(env, error))
    }

    #[napi]
    pub fn log(&self, line: String) {
        self.run.log(line);
    }

    #[napi]
    pub fn finish(&self, env: Env) -> Result<JsObject> {
        let finished = self.run.finish();
        promise(env, async move {
            let outcome = finished.await?;
            serde_json::to_string(&outcome).map_err(host_error)
        })
    }
}

/// Neo: Pending event waits do not keep an abandoned JS Run alive until its deadline.
impl Drop for Run {
    fn drop(&mut self) {
        self.run.cancel();
    }
}
