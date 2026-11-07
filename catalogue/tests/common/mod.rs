//! What the catalogue's tests share: a models folder of their own, removed afterwards, and calls
//! through the catalogue as JSON, the way an agent makes them.

#![allow(dead_code)]

use crabsim_catalogue::{Catalogue, OperationError, Settings, Source};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A folder of its own under the system's temporary folder, removed when dropped.
pub struct TempFolder {
    pub path: PathBuf,
}

impl TempFolder {
    pub fn new(name: &str) -> TempFolder {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let unique = format!("crabsim-catalogue-{name}-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed));
        let path = std::env::temp_dir().join(unique);
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempFolder { path }
    }

    pub fn join(&self, relative: &str) -> PathBuf {
        self.path.join(relative)
    }

    pub fn write(&self, relative: &str, text: &str) {
        let path = self.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    pub fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.join(relative)).unwrap()
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A models folder and a catalogue on it.
pub struct Setup {
    pub folder: TempFolder,
    pub catalogue: Catalogue,
}

impl Setup {
    pub fn new(name: &str) -> Setup {
        let folder = TempFolder::new(name);
        let catalogue = Catalogue::open(Settings::new(&folder.path, Source::Mcp)).expect("an empty folder opens");
        Setup { folder, catalogue }
    }

    /// Calls an operation that must succeed.
    pub fn ok(&self, name: &str, input: Value) -> Value {
        self.catalogue.call(name, input.clone()).unwrap_or_else(|error| panic!("{name} {input} failed: {error}"))
    }

    /// Calls an operation that must fail, giving its error as JSON, as an agent sees it.
    pub fn err(&self, name: &str, input: Value) -> Value {
        match self.catalogue.call(name, input.clone()) {
            Ok(output) => panic!("{name} {input} should fail, but gave {output}"),
            Err(error) => error.to_json(),
        }
    }

    pub fn error(&self, name: &str, input: Value) -> OperationError {
        self.catalogue.call(name, input).expect_err("should fail")
    }
}

/// The demo model's file text.
pub fn demo_text() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../models/demo.json")).unwrap()
}

/// A small valid model, in a form other than canonical.
pub const SMALL: &str = r#"{"format_version":1,"instances":[
  {"name":"In","class":"Source","successors":["Work"],"attributes":{"interval":"10"}},
  {"name":"Work","class":"Station","successors":["Out"],"attributes":{"process_time":"8"}},
  {"name":"Out","class":"Drain"}]}"#;

/// A model with three problems.
pub const BROKEN: &str = r#"{ "format_version": 1, "instances": [
    { "name": "In", "class": "Source", "successors": ["Nowhere"] },
    { "name": "Out", "class": "Drain", "attributes": { "process_time": "10" } } ] }"#;
