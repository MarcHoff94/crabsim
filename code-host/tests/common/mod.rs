//! What the tests share: code modules built from their sources for WebAssembly, once per test
//! binary.

#![allow(dead_code)]

use std::sync::OnceLock;

/// The example module `routing`, from `code-examples/routing`.
pub fn routing() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| std::fs::read(crabsim_code::testing::routing()).unwrap())
}

/// The test module `faults`, from `tests/modules/faults`.
pub fn faults() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/modules/faults");
        std::fs::read(crabsim_code::testing::build(&dir, "faults")).unwrap()
    })
}
