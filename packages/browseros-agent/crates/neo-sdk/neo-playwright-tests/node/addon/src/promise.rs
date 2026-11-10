//! Neo: Settles native futures on Node's thread, safely discarding results when an environment
//! unloads. napi 2's JsDeferred assumes its callback always receives a live environment.

use std::future::Future;
use std::ptr;
use std::sync::{Arc, Mutex};

use napi::bindgen_prelude::ToNapiValue;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::{Env, Error, JsError, JsObject, JsUnknown, NapiValue, Result, sys};
use neo_runtime::RunError;

use crate::executor;

/// Neo: Build a real JS Error on its owning thread; N-API preserves the referenced object.
pub(crate) fn native_error(env: Env, error: RunError) -> Error {
    let result = (|| {
        let mut value = env.create_error(Error::from_reason(error.message))?;
        value.set_named_property("name", error.name)?;
        Ok::<_, Error>(Error::from(value.into_unknown()))
    })();
    result.unwrap_or_else(|error| error)
}

/// Neo: Host and serialization failures have no specialized native error name.
pub(crate) fn host_error(error: impl ToString) -> RunError {
    RunError {
        name: "Error".to_string(),
        message: error.to_string(),
    }
}

/// Neo: The threadsafe function checks environment validity before touching the raw deferred.
/// Its closure owns the result slot, so abandoning a callback still drops native resources.
pub(crate) fn promise<T, F>(env: Env, future: F) -> Result<JsObject>
where
    T: ToNapiValue + Send + 'static,
    F: Future<Output = std::result::Result<T, RunError>> + Send + 'static,
{
    let executor = executor()?;
    let mut deferred = ptr::null_mut();
    let mut value = ptr::null_mut();
    // These pointers belong to this environment and are used only in its JS callback below.
    napi::check_status!(unsafe { sys::napi_create_promise(env.raw(), &mut deferred, &mut value) })?;
    let deferred = deferred as usize;
    let slot = Arc::new(Mutex::new(None::<std::result::Result<T, RunError>>));
    let delivery = Arc::clone(&slot);
    let noop =
        env.create_function_from_closure("settleHostedRun", |ctx| ctx.env.get_undefined())?;
    let settle: ThreadsafeFunction<()> = noop.create_threadsafe_function(1, move |ctx| {
        let result = delivery
            .lock()
            .map_err(|error| Error::from_reason(error.to_string()))?
            .take();
        if let Some(result) = result {
            let result = result
                .map_err(|error| native_error(ctx.env, error))
                .and_then(|value| unsafe { T::to_napi_value(ctx.env.raw(), value) });
            let status = match result {
                Ok(value) => unsafe {
                    sys::napi_resolve_deferred(ctx.env.raw(), deferred as sys::napi_deferred, value)
                },
                Err(error) => unsafe {
                    let error = JsError::from(error).into_value(ctx.env.raw());
                    sys::napi_reject_deferred(ctx.env.raw(), deferred as sys::napi_deferred, error)
                },
            };
            napi::check_status!(status)?;
        }
        Ok(Vec::<JsUnknown>::new())
    })?;
    executor.spawn(async move {
        let result = future.await;
        if let Ok(mut slot) = slot.lock() {
            *slot = Some(result);
        }
        let _ = settle.call(Ok(()), ThreadsafeFunctionCallMode::NonBlocking);
        // If Node is closing, no callback can consume the slot. Dropping the TSFN closure
        // and this Arc releases the result, including any not-yet-delivered Runtime or Run.
    });
    unsafe { JsObject::from_raw(env.raw(), value) }
}
