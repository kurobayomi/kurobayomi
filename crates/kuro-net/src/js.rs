//! JavaScript evaluation for sources (area 12, T-12a-028; `JavaScriptEngine.kt`).
//! Compiled only with the `js-sources` feature. Each call gets its own QuickJS runtime and context, with a memory
//! limit, a stack limit and a wall-clock deadline, and runs on a blocking thread so the caller's runtime is not held.

use std::time::{Duration, Instant};

use rquickjs::{CaughtError, Ctx, Runtime, Value};

/// A script result that a source can read: the primitive types of the Kotlin `as T` cast.
#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    Null,
    Bool(bool),
    Int(i32),
    Double(f64),
    Str(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsError {
    /// The script failed to parse or threw. The text is the exception message.
    Script(String),
    /// The result is not a primitive (an object, an array, a function, `undefined`, a symbol or a BigInt).
    UnsupportedResult(&'static str),
    MemoryLimit,
    Timeout,
}

/// The three limits of one evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsLimits {
    pub timeout: Duration,
    pub memory_bytes: usize,
    pub stack_bytes: usize,
}

impl Default for JsLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            memory_bytes: 64 * 1024 * 1024,
            stack_bytes: 512 * 1024,
        }
    }
}

/// Evaluates `script` with the default limits.
pub async fn evaluate(script: String) -> Result<JsValue, JsError> {
    evaluate_with(script, JsLimits::default()).await
}

/// Evaluates `script` with `limits`.
pub async fn evaluate_with(script: String, limits: JsLimits) -> Result<JsValue, JsError> {
    tokio::task::spawn_blocking(move || evaluate_blocking(&script, limits))
        .await
        .map_err(|err| JsError::Script(err.to_string()))?
}

fn evaluate_blocking(script: &str, limits: JsLimits) -> Result<JsValue, JsError> {
    let runtime = Runtime::new().map_err(|err| JsError::Script(err.to_string()))?;
    runtime.set_memory_limit(limits.memory_bytes);
    runtime.set_max_stack_size(limits.stack_bytes);
    let deadline = Instant::now() + limits.timeout;
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag_in_handler = flag.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || {
        if Instant::now() >= deadline {
            flag_in_handler.store(true, std::sync::atomic::Ordering::SeqCst);
            true
        } else {
            false
        }
    })));
    let context =
        rquickjs::Context::full(&runtime).map_err(|err| JsError::Script(err.to_string()))?;
    let outcome = context.with(|ctx| -> Result<JsValue, JsError> {
        let value: Value = match ctx.eval(script.as_bytes().to_vec()) {
            Ok(value) => value,
            Err(err) => return Err(classify(&ctx, err)),
        };
        to_js_value(&value)
    });
    if flag.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(JsError::Timeout);
    }
    outcome
}

/// Maps a failed evaluation to its error. QuickJS reports an exhausted heap as an "out of memory" exception.
fn classify(ctx: &Ctx<'_>, err: rquickjs::Error) -> JsError {
    let text = CaughtError::from_error(ctx, err).to_string();
    if text.contains("out of memory") {
        JsError::MemoryLimit
    } else {
        JsError::Script(text)
    }
}

fn to_js_value(value: &Value<'_>) -> Result<JsValue, JsError> {
    if value.is_null() {
        return Ok(JsValue::Null);
    }
    if let Some(flag) = value.as_bool() {
        return Ok(JsValue::Bool(flag));
    }
    if let Some(int) = value.as_int() {
        return Ok(JsValue::Int(int));
    }
    if let Some(float) = value.as_float() {
        return Ok(JsValue::Double(float));
    }
    if let Some(text) = value.as_string() {
        return text
            .to_string()
            .map(JsValue::Str)
            .map_err(|err| JsError::Script(err.to_string()));
    }
    let kind = if value.is_undefined() {
        "undefined"
    } else if value.is_symbol() {
        "symbol"
    } else if value.is_big_int() {
        "bigint"
    } else if value.is_function() {
        "function"
    } else if value.is_array() {
        "array"
    } else {
        "object"
    };
    Err(JsError::UnsupportedResult(kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn primitives_come_back_as_their_variant() {
        assert_eq!(evaluate("null".into()).await, Ok(JsValue::Null));
        assert_eq!(evaluate("true".into()).await, Ok(JsValue::Bool(true)));
        assert_eq!(evaluate("6 * 7".into()).await, Ok(JsValue::Int(42)));
        assert_eq!(evaluate("1.5 + 1".into()).await, Ok(JsValue::Double(2.5)));
        assert_eq!(
            evaluate("'ab' + 'c'".into()).await,
            Ok(JsValue::Str("abc".into()))
        );
    }

    #[tokio::test]
    async fn an_object_is_an_unsupported_result_not_a_cast_error() {
        assert_eq!(
            evaluate("({a: 1})".into()).await,
            Err(JsError::UnsupportedResult("object"))
        );
        assert_eq!(
            evaluate("[1, 2]".into()).await,
            Err(JsError::UnsupportedResult("array"))
        );
        assert_eq!(
            evaluate("(() => 1)".into()).await,
            Err(JsError::UnsupportedResult("function"))
        );
        assert_eq!(
            evaluate("undefined".into()).await,
            Err(JsError::UnsupportedResult("undefined"))
        );
    }

    #[tokio::test]
    async fn a_thrown_error_is_a_script_error() {
        let err = evaluate("throw new Error('boom')".into())
            .await
            .unwrap_err();
        assert!(
            matches!(err, JsError::Script(ref text) if text.contains("boom")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn a_script_that_never_ends_is_aborted_at_the_deadline() {
        let limits = JsLimits {
            timeout: Duration::from_secs(2),
            ..JsLimits::default()
        };
        let started = Instant::now();
        let result = evaluate_with("while (true) {}".into(), limits).await;
        assert_eq!(result, Err(JsError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[tokio::test]
    async fn memory_above_the_limit_aborts_with_memory_limit() {
        let script = "var a = []; while (true) { a.push('x'.repeat(1 << 20)); }".to_owned();
        assert_eq!(evaluate(script).await, Err(JsError::MemoryLimit));
    }
}
