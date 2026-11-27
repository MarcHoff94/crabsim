//! crabsim's functions as numbers: imported from `crabsim` in WebAssembly, or called on the host
//! directly in native mode. Everything above this is the same on both.

#[cfg(target_arch = "wasm32")]
mod imports {
    #[link(wasm_import_module = "crabsim")]
    unsafe extern "C" {
        pub fn now() -> i64;
        pub fn object_count() -> i32;
        pub fn object_kind(o: i32) -> i32;
        pub fn object_name(o: i32, ptr: i32, cap: i32) -> i32;
        pub fn find_object(ptr: i32, len: i32) -> i32;
        pub fn successor_count(o: i32) -> i32;
        pub fn successor(o: i32, i: i32) -> i32;
        pub fn predecessor_count(o: i32) -> i32;
        pub fn predecessor(o: i32, i: i32) -> i32;
        pub fn process_time(o: i32) -> i64;
        pub fn object_state(o: i32) -> i32;
        pub fn mus_held(o: i32) -> i32;
        pub fn content(o: i32, i: i32) -> i64;
        pub fn statistic(o: i32, which: i32) -> f64;
        pub fn mu_kind(mu: i32) -> i32;
        pub fn mu_class_name(kind: i32, ptr: i32, cap: i32) -> i32;
        pub fn mu_location(mu: i32) -> i32;
        pub fn mu_left_source(mu: i32) -> i64;
        pub fn param_int(i: i32) -> i64;
        pub fn param_float(i: i32) -> f64;
        pub fn param_text(i: i32, ptr: i32, cap: i32) -> i32;
        pub fn random(o: i32) -> f64;
        pub fn move_mu(mu: i32, to: i32) -> i32;
        pub fn place_mu(mu: i32, to: i32, at: f64, point: i32) -> i32;
        pub fn set_process_time(o: i32, millis: i64) -> i32;
        pub fn set_work_time(o: i32, mu: i32, millis: i64) -> i32;
        pub fn pause(o: i32, on: i32) -> i32;
        pub fn stop(o: i32, on: i32) -> i32;
        pub fn call_at(after: i64, data: i32) -> i32;
        pub fn refusal(ptr: i32, cap: i32) -> i32;
        pub fn log(ptr: i32, len: i32);
        pub fn fail(ptr: i32, len: i32);
    }
}

/// Reads a text crabsim writes into a buffer, asking again with more room when it doesn't fit.
#[cfg(target_arch = "wasm32")]
fn text(fill: impl Fn(i32, i32) -> i32) -> Option<String> {
    let mut buffer = vec![0u8; 64];
    loop {
        let length = fill(buffer.as_mut_ptr() as i32, buffer.len() as i32);
        if length < 0 {
            return None;
        }
        let length = length as usize;
        if length <= buffer.len() {
            buffer.truncate(length);
            return Some(String::from_utf8_lossy(&buffer).into_owned());
        }
        buffer.resize(length, 0);
    }
}

#[cfg(target_arch = "wasm32")]
macro_rules! numbers {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        $(pub fn $name($($arg: $ty),*) -> $ret {
            // SAFETY: crabsim's imports take and give numbers only.
            unsafe { imports::$name($($arg),*) }
        })*
    };
}

#[cfg(not(target_arch = "wasm32"))]
macro_rules! numbers {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        $(pub fn $name($($arg: $ty),*) -> $ret {
            native::$name($($arg),*)
        })*
    };
}

numbers! {
    now() -> i64;
    object_count() -> i32;
    object_kind(o: i32) -> i32;
    successor_count(o: i32) -> i32;
    successor(o: i32, i: i32) -> i32;
    predecessor_count(o: i32) -> i32;
    predecessor(o: i32, i: i32) -> i32;
    process_time(o: i32) -> i64;
    object_state(o: i32) -> i32;
    mus_held(o: i32) -> i32;
    content(o: i32, i: i32) -> i64;
    statistic(o: i32, which: i32) -> f64;
    mu_kind(mu: i32) -> i32;
    mu_location(mu: i32) -> i32;
    mu_left_source(mu: i32) -> i64;
    param_int(i: i32) -> i64;
    param_float(i: i32) -> f64;
    random(o: i32) -> f64;
    move_mu(mu: i32, to: i32) -> i32;
    place_mu(mu: i32, to: i32, at: f64, point: i32) -> i32;
    set_process_time(o: i32, millis: i64) -> i32;
    set_work_time(o: i32, mu: i32, millis: i64) -> i32;
    pause(o: i32, on: i32) -> i32;
    stop(o: i32, on: i32) -> i32;
    call_at(after: i64, data: i32) -> i32;
}

#[cfg(target_arch = "wasm32")]
pub fn object_name(o: i32) -> Option<String> {
    text(|ptr, cap| unsafe { imports::object_name(o, ptr, cap) })
}

#[cfg(target_arch = "wasm32")]
pub fn find_object(name: &str) -> i32 {
    unsafe { imports::find_object(name.as_ptr() as i32, name.len() as i32) }
}

#[cfg(target_arch = "wasm32")]
pub fn mu_class_name(kind: i32) -> Option<String> {
    text(|ptr, cap| unsafe { imports::mu_class_name(kind, ptr, cap) })
}

#[cfg(target_arch = "wasm32")]
pub fn param_text(i: i32) -> String {
    text(|ptr, cap| unsafe { imports::param_text(i, ptr, cap) }).unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
pub fn refusal() -> String {
    text(|ptr, cap| unsafe { imports::refusal(ptr, cap) }).unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
pub fn log(message: &str) {
    unsafe { imports::log(message.as_ptr() as i32, message.len() as i32) }
}

/// Tells crabsim what the code panicked with, before it traps.
#[cfg(target_arch = "wasm32")]
pub fn fail(message: &str) {
    unsafe { imports::fail(message.as_ptr() as i32, message.len() as i32) }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{find_object, log, mu_class_name, object_name, param_text, refusal};

/// Native mode: the same functions on the host of the call in progress.
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use crabsim_code::native::with_host;

    fn id(n: i32) -> u32 {
        n as u32
    }

    fn found(id: Option<u32>) -> i32 {
        id.map_or(-1, |id| id as i32)
    }

    pub fn now() -> i64 {
        with_host(|host| host.now())
    }
    pub fn object_count() -> i32 {
        with_host(|host| host.object_count() as i32)
    }
    pub fn object_kind(o: i32) -> i32 {
        with_host(|host| host.object_kind(id(o)))
    }
    pub fn object_name(o: i32) -> Option<String> {
        with_host(|host| host.object_name(id(o)).map(str::to_string))
    }
    pub fn find_object(name: &str) -> i32 {
        with_host(|host| found(host.find_object(name)))
    }
    pub fn successor_count(o: i32) -> i32 {
        with_host(|host| host.successors(id(o)).len() as i32)
    }
    pub fn successor(o: i32, i: i32) -> i32 {
        with_host(|host| found(host.successors(id(o)).get(i as u32 as usize).copied()))
    }
    pub fn predecessor_count(o: i32) -> i32 {
        with_host(|host| host.predecessors(id(o)).len() as i32)
    }
    pub fn predecessor(o: i32, i: i32) -> i32 {
        with_host(|host| found(host.predecessors(id(o)).get(i as u32 as usize).copied()))
    }
    pub fn process_time(o: i32) -> i64 {
        with_host(|host| host.process_time(id(o)).unwrap_or(-1))
    }
    pub fn object_state(o: i32) -> i32 {
        with_host(|host| host.object_state(id(o)))
    }
    pub fn mus_held(o: i32) -> i32 {
        with_host(|host| host.mus_held(id(o)) as i32)
    }
    pub fn content(o: i32, i: i32) -> i64 {
        with_host(|host| host.content(id(o), i as u32).map_or(-1, i64::from))
    }
    pub fn statistic(o: i32, which: i32) -> f64 {
        with_host(|host| host.statistic(id(o), which))
    }
    pub fn mu_kind(mu: i32) -> i32 {
        with_host(|host| found(host.mu_kind(id(mu))))
    }
    pub fn mu_class_name(kind: i32) -> Option<String> {
        with_host(|host| host.mu_class_name(id(kind)).map(str::to_string))
    }
    pub fn mu_location(mu: i32) -> i32 {
        with_host(|host| found(host.mu_location(id(mu))))
    }
    pub fn mu_left_source(mu: i32) -> i64 {
        with_host(|host| host.mu_left_source(id(mu)).unwrap_or(-1))
    }
    pub fn param_int(i: i32) -> i64 {
        with_host(|host| host.param_int(i as u32))
    }
    pub fn param_float(i: i32) -> f64 {
        with_host(|host| host.param_float(i as u32))
    }
    pub fn param_text(i: i32) -> String {
        with_host(|host| host.param_text(i as u32).to_string())
    }
    pub fn random(o: i32) -> f64 {
        with_host(|host| host.random(id(o)))
    }
    pub fn move_mu(mu: i32, to: i32) -> i32 {
        with_host(|host| host.move_mu(id(mu), id(to)) as i32)
    }
    pub fn place_mu(mu: i32, to: i32, at: f64, point: i32) -> i32 {
        with_host(|host| host.place_mu(id(mu), id(to), at, point) as i32)
    }
    pub fn set_process_time(o: i32, millis: i64) -> i32 {
        with_host(|host| host.set_process_time(id(o), millis) as i32)
    }
    pub fn set_work_time(o: i32, mu: i32, millis: i64) -> i32 {
        with_host(|host| host.set_work_time(id(o), id(mu), millis) as i32)
    }
    pub fn pause(o: i32, on: i32) -> i32 {
        with_host(|host| host.pause(id(o), on != 0) as i32)
    }
    pub fn stop(o: i32, on: i32) -> i32 {
        with_host(|host| host.stop(id(o), on != 0) as i32)
    }
    pub fn call_at(after: i64, data: i32) -> i32 {
        with_host(|host| host.call_at(after, data as u32) as i32)
    }
    pub fn refusal() -> String {
        with_host(|host| host.refusal().to_string())
    }
    pub fn log(message: &str) {
        with_host(|host| host.log(message))
    }
}
