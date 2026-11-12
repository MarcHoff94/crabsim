//! The models folder: the one place crabsim's operations may touch. Model files are named by their
//! path inside it, and nothing an operation is given can reach outside it, through `..`, an
//! absolute path or a link, or into crabsim's own `.crabsim` folder there, or into `exports`,
//! where `export_model` alone writes, and only graph files.

use crate::error::{ErrorKind, OperationError};
use std::path::{Component, Path, PathBuf};

/// crabsim's own folder inside the models folder, holding the results store.
pub const CRABSIM_FOLDER: &str = ".crabsim";

/// The folder inside the models folder where `export_model` writes. Model operations can't reach
/// it, and nothing else is written there.
pub const EXPORTS_FOLDER: &str = "exports";

/// The only kinds of file written in `exports`.
const EXPORT_EXTENSIONS: [&str; 3] = ["cypher", "csv", "graphml"];

/// A models folder that passed the check for source code.
#[derive(Debug, Clone)]
pub struct ModelsFolder {
    /// Canonical, so every path inside it can be compared with it after following links.
    root: PathBuf,
}

impl ModelsFolder {
    /// Opens `folder` as a models folder. Refused if it isn't a folder, or if it holds a
    /// `Cargo.toml`, a `.git` or any `.rs` file at any depth, so crabsim's operations can't be
    /// pointed at a source tree, such as crabsim's own repository, by mistake.
    pub fn open(folder: &Path) -> Result<ModelsFolder, OperationError> {
        let root = std::fs::canonicalize(folder)
            .map_err(|error| OperationError::not_found(format!("the models folder {} can't be opened: {error}", folder.display())))?;
        if !root.is_dir() {
            return Err(OperationError::invalid_input(format!("the models folder {} isn't a folder", folder.display())));
        }
        if let Some(found) = source_code_in(&root) {
            return Err(OperationError::invalid_input(format!(
                "{} looks like source code, not a models folder: it holds {}. Choose a folder of models without code",
                folder.display(),
                found.display()
            )));
        }
        Ok(ModelsFolder { root })
    }

    /// The folder, canonical.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// crabsim's own folder inside it.
    pub fn crabsim_folder(&self) -> PathBuf {
        self.root.join(CRABSIM_FOLDER)
    }

    /// The model file at `path` inside the folder, which may not exist yet. `path` must be
    /// relative, stay inside the folder after following links, end in `.json` and keep out of
    /// `.crabsim`.
    pub fn model_file(&self, path: &str) -> Result<PathBuf, OperationError> {
        let relative = relative_path(path)?;
        // The macros live beside the models, but aren't one.
        if relative == std::path::Path::new(crate::macros::MACROS_FILE) {
            return Err(OperationError::invalid_input(format!("`{path}` holds the macros, which list_macros, save_macro and remove_macro change: it isn't a model file")));
        }
        if !relative.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("json")) {
            return Err(OperationError::invalid_input(format!("`{path}` isn't a model file: model files end in .json")));
        }
        self.inside(path, &relative)
    }

    /// The subfolder at `path` inside the folder, or the folder itself for an empty path.
    pub fn subfolder(&self, path: &str) -> Result<PathBuf, OperationError> {
        let relative = relative_path(path)?;
        self.inside(path, &relative)
    }

    /// The export file at `path` inside `exports`, which may not exist yet. `path` must be
    /// relative, end in `.cypher`, `.csv` or `.graphml`, and stay inside `exports` after following
    /// links.
    pub fn export_file(&self, path: &str) -> Result<PathBuf, OperationError> {
        let relative = relative_path(path)?;
        if !relative.extension().is_some_and(|extension| EXPORT_EXTENSIONS.iter().any(|allowed| extension.eq_ignore_ascii_case(allowed))) {
            return Err(OperationError::invalid_input(format!("`{path}` isn't an export: exports end in .cypher, .csv or .graphml")));
        }
        let full = self.root.join(EXPORTS_FOLDER).join(&relative);
        let named = format!("{EXPORTS_FOLDER}/{path}");
        let real = self.nearest_real(&named, &full)?;
        // Before the first export there is no `exports` yet, so the nearest real folder is the
        // models folder itself. Otherwise it must be `exports` or inside it, not a folder a link
        // in `exports` leads to.
        let Ok(inside) = real.strip_prefix(&self.root) else { return Err(outside(&named)) };
        let first = inside.components().next();
        if first.is_some_and(|first| !is_exports_folder(first.as_os_str())) {
            return Err(outside(&named));
        }
        Ok(full)
    }

    /// The nearest part of `full` that exists, followed through any links.
    fn nearest_real(&self, path: &str, full: &Path) -> Result<PathBuf, OperationError> {
        let mut existing = full;
        loop {
            match std::fs::canonicalize(existing) {
                Ok(real) => return Ok(real),
                Err(_) => match existing.parent() {
                    Some(parent) => existing = parent,
                    None => return Err(outside(path)),
                },
            }
        }
    }

    /// `relative` joined to the folder, checked to stay inside it once links are followed.
    fn inside(&self, path: &str, relative: &Path) -> Result<PathBuf, OperationError> {
        // As given, so a path into `exports` is refused before it exists too, and after
        // following links.
        if relative.components().next().is_some_and(|first| is_exports_folder(first.as_os_str())) {
            return Err(outside(path));
        }
        let full = self.root.join(relative);
        let real = self.nearest_real(path, &full)?;
        let Ok(inside) = real.strip_prefix(&self.root) else { return Err(outside(path)) };
        if inside.components().next().is_some_and(|first| is_crabsim_folder(first.as_os_str()) || is_exports_folder(first.as_os_str())) {
            return Err(outside(path));
        }
        Ok(full)
    }

    /// `full`, a path inside the folder, as an operation names it: relative, with `/` between
    /// parts.
    pub fn name_of(&self, full: &Path) -> String {
        let relative = full.strip_prefix(&self.root).unwrap_or(full);
        relative.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
    }
}

/// `path` as a relative path made only of plain names, or why not.
fn relative_path(path: &str) -> Result<PathBuf, OperationError> {
    let given = Path::new(path);
    let mut relative = PathBuf::new();
    for part in given.components() {
        match part {
            Component::Normal(name) if is_crabsim_folder(name) => return Err(outside(path)),
            Component::Normal(name) => relative.push(name),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return Err(outside(path)),
        }
    }
    // A Windows path such as `C:model.json` has a prefix but no root, so check both ways.
    if given.is_absolute() || given.has_root() {
        return Err(outside(path));
    }
    Ok(relative)
}

fn is_crabsim_folder(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy().eq_ignore_ascii_case(CRABSIM_FOLDER)
}

fn is_exports_folder(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy().eq_ignore_ascii_case(EXPORTS_FOLDER)
}

fn outside(path: &str) -> OperationError {
    OperationError::new(
        ErrorKind::OutsideModelsFolder,
        format!("`{path}` is outside the models folder: name a path inside it, such as `line.json` or `plant/line.json`, without `..`, and not in `.crabsim` or `exports`, which hold crabsim's own files and exports"),
    )
}

/// The first sign of source code in `folder`: a `Cargo.toml`, a `.git` or a `.rs` file at any
/// depth. Links aren't followed, so a link can't make the search loop.
fn source_code_in(folder: &Path) -> Option<PathBuf> {
    let mut folders = vec![folder.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name == "cargo.toml" || name == ".git" || name.ends_with(".rs") {
                return Some(path);
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) && name != CRABSIM_FOLDER {
                folders.push(path);
            }
        }
    }
    None
}

