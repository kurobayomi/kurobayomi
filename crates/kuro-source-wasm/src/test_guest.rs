//! WAT guests for the host tests. Each one is a component whose `invoke` does one thing: return text, return an error,
//! trap, spin, or grow memory. They are text, so the tests need no toolchain and no binary fixtures.

use std::fmt::Write;

/// What the guest's `invoke` does.
pub enum Behaviour {
    /// Returns `Ok(text)` for every op.
    Returns(String),
    /// Returns `Err(text)` for every op.
    Fails(String),
    /// Returns its own `input-json` as the `Ok` text.
    Echoes,
    /// Traps at once.
    Traps,
    /// Loops forever, so only the epoch deadline stops it.
    Spins,
    /// Grows memory until the store refuses, then traps.
    Grows,
    /// Sends `times` GET requests to `url` through the host `http` import, then answers its descriptor for every op. With
    /// `then_spin` it loops forever after the requests, so only the call deadline stops it.
    Fetches { url: String, times: u32, then_spin: bool },
}

/// The component bytes (WAT text, which `Component::new` accepts with the `wat` feature).
pub fn guest(behaviour: Behaviour) -> Vec<u8> {
    let (payload, body) = match behaviour {
        Behaviour::Returns(text) => (text, returns(0)),
        Behaviour::Fails(text) => (text, returns(1)),
        Behaviour::Echoes => (String::new(), echoes()),
        Behaviour::Traps => (String::new(), "(unreachable)".to_owned()),
        Behaviour::Spins => (String::new(), "(loop $spin (br $spin))".to_owned()),
        Behaviour::Grows => (String::new(), grows()),
        Behaviour::Fetches { url, times, then_spin } => return fetching(&url, times, then_spin),
    };
    template(&payload, &body).into_bytes()
}

/// The `host` import of ABI 1.4, as a guest built for 1.4 declares it.
pub const IMPORTS_HOST_V14: &str = r#"(import "kuro:source/host@1.4.0" (instance
    (export "http" (func (param "request-json" string) (result (result string (error string)))))
    (export "pref-get" (func (param "key" string) (result string)))
    (export "pref-set" (func (param "key" string) (param "value-json" string) (result (result (error string)))))
    (export "log" (func (param "level" u8) (param "message" string)))))"#;

/// The `host` import of ABI 1.5: the 1.4 functions and the clock.
pub const IMPORTS_HOST_V15: &str = r#"(import "kuro:source/host@1.5.0" (instance
    (export "http" (func (param "request-json" string) (result (result string (error string)))))
    (export "pref-get" (func (param "key" string) (result string)))
    (export "pref-set" (func (param "key" string) (param "value-json" string) (result (result (error string)))))
    (export "log" (func (param "level" u8) (param "message" string)))
    (export "unix-seconds" (func (result u64)))))"#;

/// The `pages` import, which a guest that reads web pages declares next to `host`.
pub const IMPORTS_PAGES: &str = r#"(import "kuro:source/pages@1.5.0" (instance
    (export "read" (func (param "request-json" string) (result (result string (error string)))))))"#;

/// A guest that declares `import` (one of the `IMPORTS_*` texts) and answers `text` for every op, like [`guest`].
pub fn guest_importing(import: &str, text: &str) -> Vec<u8> {
    template(text, &returns(0)).replacen("(component", &format!("(component {import}"), 1).into_bytes()
}

/// A component that exports nothing the host calls. Loading it must fail with the missing-export error.
pub fn without_invoke() -> Vec<u8> {
    b"(component (core module $m (memory (export \"memory\") 1)))".to_vec()
}

/// Writes `Ok` (`discriminant` 0) or `Err` (1) with the payload at offset 0 into the return area.
fn returns(discriminant: u8) -> String {
    format!(
        "(i32.store (i32.const {RET_AREA}) (i32.const {discriminant}))\n\
         (i32.store offset=4 (i32.const {RET_AREA}) (i32.const 0))\n\
         (i32.store offset=8 (i32.const {RET_AREA}) (i32.const PAYLOAD_LEN))"
    )
}

/// Returns the input string as the `Ok` text. The lifted string points into memory the host wrote.
fn echoes() -> String {
    format!(
        "(i32.store (i32.const {RET_AREA}) (i32.const 0))\n\
         (i32.store offset=4 (i32.const {RET_AREA}) (local.get 2))\n\
         (i32.store offset=8 (i32.const {RET_AREA}) (local.get 3))"
    )
}

fn grows() -> String {
    "(block $done
       (loop $grow
         (br_if $done (i32.eq (memory.grow (i32.const 16)) (i32.const -1)))
         (br $grow)))
     (unreachable)"
        .to_owned()
}

/// Where the guest writes its `result<string, string>`. The payload sits at offset 0, and the host's allocations start
/// at 128 KiB, so this area lies between them.
const RET_AREA: u32 = 65536;

/// The descriptor a fetching guest answers with: one source, no request gap.
const FETCHING_DESCRIPTOR: &str = r#"{"id":1,"name":"Fetcher","lang":"en"}"#;
/// Where a fetching guest keeps its request JSON: after the descriptor at 0, before the host's allocations.
const REQUEST_AT: u32 = 1024;
/// Where the result of the guest's own `http` call is written, apart from the export's [`RET_AREA`].
const HTTP_RET_AREA: u32 = 65600;

/// The bytes of `text` as the hex escapes of a WAT string.
fn escaped(text: &str) -> String {
    let mut data = String::new();
    for byte in text.bytes() {
        write!(data, "\\{byte:02x}").expect("writing to a String");
    }
    data
}

/// A guest that sends `times` GET requests to `url` through the host `http` import, then answers its descriptor.
fn fetching(url: &str, times: u32, then_spin: bool) -> Vec<u8> {
    let request = format!(r#"{{"method":"GET","url":"{url}","headers":[],"body":null}}"#);
    let spin = if then_spin { "(loop $spin (br $spin))" } else { "" };
    let body = format!(
        "(local $i i32)
         (block $done
           (loop $again
             (br_if $done (i32.ge_u (local.get $i) (i32.const {times})))
             (call $http (i32.const {REQUEST_AT}) (i32.const {request_len}) (i32.const {HTTP_RET_AREA}))
             (local.set $i (i32.add (local.get $i) (i32.const 1)))
             (br $again)))
         {spin}
         {ret}",
        request_len = request.len(),
        ret = returns(0),
    );
    fetching_template(FETCHING_DESCRIPTOR, &request, &body).into_bytes()
}

/// The component of a fetching guest. The host `http` function is lowered against the memory and the realloc of a memory
/// module, which the guest's own module then imports, because the lowered function cannot exist before that memory does.
fn fetching_template(descriptor: &str, request: &str, body: &str) -> String {
    let body = body.replace("PAYLOAD_LEN", &descriptor.len().to_string());
    format!(
        r#"(component
  (import "kuro:source/host@1.5.0" (instance $host
    (export "http" (func (param "request-json" string) (result (result string (error string)))))))
  (alias export $host "http" (func $http_import))
  (core module $memory_module
    (memory (export "memory") 4)
    (global $heap (mut i32) (i32.const 131072))
    (data (i32.const 0) "{descriptor_bytes}")
    (data (i32.const {REQUEST_AT}) "{request_bytes}")
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
      (local $p i32)
      (local.set $p (global.get $heap))
      (global.set $heap (i32.add (local.get $p) (local.get 3)))
      (local.get $p)))
  (core instance $memory_instance (instantiate $memory_module))
  (alias core export $memory_instance "memory" (core memory $shared))
  (alias core export $memory_instance "cabi_realloc" (core func $realloc))
  (core func $http_lower (canon lower (func $http_import) (memory $shared) (realloc (func $realloc))))
  (core instance $host_core (export "http" (func $http_lower)))
  (core module $guest
    (import "mem" "memory" (memory 4))
    (import "host" "http" (func $http (param i32 i32 i32)))
    (func (export "invoke") (param i32 i32 i32 i32) (result i32)
      {body}
      (i32.const {RET_AREA})))
  (core instance $guest_instance (instantiate $guest
    (with "mem" (instance $memory_instance))
    (with "host" (instance $host_core))))
  (alias core export $guest_instance "invoke" (core func $invoke))
  (func (export "invoke") (param "op" string) (param "input-json" string) (result (result string (error string)))
    (canon lift (core func $invoke) (memory $shared) (realloc (func $realloc))))
)
"#,
        descriptor_bytes = escaped(descriptor),
        request_bytes = escaped(request),
    )
}

/// The component around one `invoke` body. A lifted export with a multi-value result returns a pointer to that result.
fn template(payload: &str, body: &str) -> String {
    let len = payload.len();
    let body = body.replace("PAYLOAD_LEN", &len.to_string());
    let data = escaped(payload);
    format!(
        r#"(component
  (core module $m
    (memory (export "memory") 4)
    (global $heap (mut i32) (i32.const 131072))
    (data (i32.const 0) "{data}")
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
      (local $p i32)
      (local.set $p (global.get $heap))
      (global.set $heap (i32.add (local.get $p) (local.get 3)))
      (local.get $p))
    (func (export "invoke") (param i32 i32 i32 i32) (result i32)
      {body}
      (i32.const {RET_AREA})))
  (core instance $i (instantiate $m))
  (alias core export $i "memory" (core memory $mem))
  (alias core export $i "cabi_realloc" (core func $realloc))
  (alias core export $i "invoke" (core func $invoke))
  (func (export "invoke") (param "op" string) (param "input-json" string) (result (result string (error string)))
    (canon lift (core func $invoke) (memory $mem) (realloc (func $realloc))))
)
"#
    )
}

/// The environment every test guest gets: an in-memory preference store, and a client that makes no request on its own.
/// The directory holds the cookie file, so the caller keeps it alive for the test.
pub fn env() -> (kuro_source_wasm::source::HostEnv, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let prefs = kuro_prefs::network::NetworkPrefs::new(kuro_prefs::store::PrefStore::in_memory());
    let (toaster, _feed) = kuro_platform::toast::toast_channel();
    let client = kuro_net::NetworkClient::new(
        &prefs,
        dir.path().join("cookies.json"),
        std::sync::Arc::new(kuro_platform::webview::NoWebView),
        toaster,
    )
    .expect("client");
    let host_env = kuro_source_wasm::source::HostEnv {
        client,
        prefs: kuro_prefs::store::PrefStore::in_memory(),
    };
    (host_env, dir)
}
