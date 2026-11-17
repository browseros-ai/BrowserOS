//! The product's string-only Bridge door, hosted in Node. No browser behavior lives here.

mod promise;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use napi::{Env, Error, JsObject, Result};
use napi_derive::napi;
use neo_server::App;
use neo_server::hosted::{HostedOptions, HostedRun, HostedRuntime};
use tokio::runtime::Runtime as TokioRuntime;

static EXECUTOR: OnceLock<std::result::Result<TokioRuntime, String>> = OnceLock::new();
const DEFAULT_TIMEOUT_MS: u32 = 24 * 60 * 60 * 1000;

// This executor only settles N-API futures. Each HostedRuntime owns the server's background
// tasks on a separate App::runtime(), so close releases every task and socket for that browser.
fn executor() -> Result<&'static TokioRuntime> {
    EXECUTOR
        .get_or_init(|| App::runtime().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|message| Error::from_reason(message.clone()))
}

#[napi(object)]
#[derive(Default)]
pub struct ConnectOptions {
    pub state_dir: String,
    pub execution_dir: Option<String>,
    pub browseros_version: Option<String>,
    /// Whether the browser was launched with `BROWSERCLAW_HOSTED_MODE=1`; off when absent.
    pub hosted_mode: Option<bool>,
}

#[napi]
pub struct Runtime {
    runtime: Arc<HostedRuntime>,
}

#[napi]
pub fn connect(env: Env, cdp_port: u16, options: ConnectOptions) -> Result<JsObject> {
    promise::promise(env, async move {
        let runtime = HostedRuntime::connect(
            cdp_port,
            HostedOptions {
                state_dir: PathBuf::from(options.state_dir),
                // Upstream specs read fixtures and outputs across several directories.
                // The embedding's own-files protection still applies with `/` allowed.
                read_dirs: Some(vec![PathBuf::from("/")]),
                execution_dir: options.execution_dir.map(PathBuf::from),
                browseros_version: options.browseros_version,
                hosted_mode: options.hosted_mode.unwrap_or(false),
            },
        )
        .await
        .map_err(|error| error.to_string())?;
        Ok(Runtime {
            runtime: Arc::new(runtime),
        })
    })
}

#[napi(object)]
#[derive(Default)]
pub struct OpenRunOptions {
    pub timeout_ms: Option<u32>,
}

#[napi]
impl Runtime {
    #[napi]
    pub fn open_run(&self, env: Env, options: Option<OpenRunOptions>) -> Result<JsObject> {
        let runtime = Arc::clone(&self.runtime);
        let timeout = options
            .and_then(|options| options.timeout_ms)
            .unwrap_or(DEFAULT_TIMEOUT_MS);
        promise::promise(env, async move {
            let run = runtime
                .open_run(Duration::from_millis(u64::from(timeout)))
                .await
                .map_err(|error| error.to_string())?;
            Ok(Run { run })
        })
    }

    #[napi]
    pub fn close(&self, env: Env) -> Result<JsObject> {
        let runtime = Arc::clone(&self.runtime);
        promise::promise(env, async move {
            runtime.close().await.map_err(|error| error.to_string())
        })
    }
}

// GC and environment unload still close the host; the JS object is never retained by a pending
// event wait. Native work owns only the resources needed to produce its answer.
impl Drop for Runtime {
    fn drop(&mut self) {
        if let Ok(executor) = executor() {
            let runtime = Arc::clone(&self.runtime);
            executor.spawn(async move {
                let _ = runtime.close().await;
            });
        }
    }
}

#[napi]
pub struct Run {
    run: HostedRun,
}

#[napi]
impl Run {
    #[napi(getter)]
    pub fn id(&self) -> String {
        self.run.id()
    }

    #[napi(getter)]
    pub fn init(&self) -> String {
        self.run.init().to_owned()
    }

    #[napi]
    pub fn invoke(
        &self,
        env: Env,
        name: String,
        input_json: String,
        site: String,
    ) -> Result<JsObject> {
        // This MUST stay outside the future: HostedRun enters its executor and calls
        // Bridge::submit now, before JavaScript can issue its next call or finish the run.
        let answer = self.run.invoke(name, &input_json, &site);
        promise::promise(env, async move { Ok(answer.await) })
    }

    #[napi]
    pub fn log(&self, level: String, text: String) {
        self.run.log(&level, text);
    }

    #[napi]
    pub fn sleep(&self, env: Env, milliseconds: f64) -> Result<JsObject> {
        let sleep = self.run.sleep(milliseconds);
        promise::promise(env, async move {
            sleep.await;
            Ok(())
        })
    }

    #[napi]
    pub fn finish(&self, env: Env) -> Result<JsObject> {
        let finished = self.run.finish();
        promise::promise(env, async move {
            finished.await.map_err(|error| error.to_string())
        })
    }
}
