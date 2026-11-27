//! A model's user code (ADR 0029): the code modules its hooks name, loaded from the models
//! folder's `code/` folder, checked against what each function declares, and built into the
//! engine's hooks.

use crate::{CodeCall, DistributionStrategy, Model, Problem, escape};
use crabsim::hooks::{Hook, ObjectHooks, Triggers};
use crabsim::{ModelDescription, TriggerPoint};
use crabsim_code::{Binding, Code, Library, Options};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// The folder of a models folder that holds its code modules.
pub const CODE_FOLDER: &str = "code";

/// A call of code in the file, where it is and whose it is.
struct Named<'m> {
    call: &'m CodeCall,
    /// Its pointer, up to and including `code`.
    pointer: String,
    name: Option<String>,
}

impl Model {
    /// Whether any hook names code.
    pub fn has_code(&self) -> bool {
        !self.code_calls().is_empty()
    }

    /// The code modules the hooks name, in the order first named.
    pub fn code_modules(&self) -> Vec<String> {
        let mut modules: Vec<String> = Vec::new();
        for named in self.code_calls() {
            if let Some((module, _)) = named.call.function.split_once('.')
                && !modules.iter().any(|known| known == module)
            {
                modules.push(module.to_string());
            }
        }
        modules
    }

    /// Every call of code where it is written, on classes, instances and the model, in file order.
    fn code_calls(&self) -> Vec<Named<'_>> {
        let mut calls = Vec::new();
        for (index, class) in self.classes.iter().enumerate() {
            push_calls(&mut calls, &class.attributes, format!("/classes/{index}"), &class.name);
        }
        for (index, instance) in self.instances.iter().enumerate() {
            push_calls(&mut calls, &instance.attributes, format!("/instances/{index}"), &instance.name);
        }
        for (key, _, call) in self.hooks.each() {
            if let Some(call) = call {
                calls.push(Named { call, pointer: format!("/hooks/{key}/code"), name: None });
            }
        }
        calls
    }

    /// Loads the code modules the hooks name from `<models_folder>/code/`, with a problem where
    /// each one that can't be loaded is first named.
    pub fn load_code(&self, models_folder: &Path) -> (Library, Vec<Problem>) {
        let folder = models_folder.join(CODE_FOLDER);
        let mut library = Library::new();
        let mut problems = Vec::new();
        let calls = self.code_calls();
        for module in self.code_modules() {
            if let Err(message) = library.load_from(&folder, &module) {
                let named = calls.iter().find(|named| named.call.function.split_once('.').is_some_and(|(m, _)| m == module)).expect("a module is named");
                problems.push(Problem { pointer: format!("{}/function", named.pointer), name: named.name.clone(), message });
            }
        }
        (library, problems)
    }

    /// What is wrong with the hooks against the code: functions a module doesn't have, and
    /// parameter values that don't fit, each where it is written. A module the library lacks is
    /// a problem too.
    pub fn code_problems(&self, library: &Library) -> Vec<Problem> {
        let ids = self.instance_ids();
        let mut problems = Vec::new();
        for named in self.code_calls() {
            if let Err(found) = library.bind(&named.call.function, &named.call.with, &|name| ids.get(name).copied()) {
                for problem in found {
                    let pointer = match &problem.parameter {
                        Some(parameter) if named.call.with.contains_key(parameter) => format!("{}/with/{}", named.pointer, escape(parameter)),
                        Some(_) => format!("{}/with", named.pointer),
                        None => format!("{}/function", named.pointer),
                    };
                    problems.push(Problem { pointer, name: named.name.clone(), message: problem.message });
                }
            }
        }
        problems
    }

    fn instance_ids(&self) -> HashMap<&str, u32> {
        self.instances.iter().enumerate().map(|(id, instance)| (instance.name.as_str(), id as u32)).collect()
    }

    /// The model built with its code, which `library` holds: every hook bound to its function,
    /// each instance's hooks worked out through its classes. A model without code builds as
    /// `to_description` does.
    ///
    /// # Panics
    /// If the model has problems (`Model::load` and `check` report them).
    pub fn to_description_with_code(&self, library: Arc<Library>, options: Options) -> Result<ModelDescription, Vec<Problem>> {
        let problems = self.check();
        assert!(problems.is_empty(), "the model has problems: {problems:?}");
        let (mut description, mu_classes) = crate::build::build_naming_mu_kinds(self);
        if !self.has_code() {
            return Ok(description);
        }
        let problems = self.code_problems(&library);
        if !problems.is_empty() {
            return Err(problems);
        }
        let ids = self.instance_ids();
        // One hook function per call and its values, however many objects share it.
        let mut bindings: Vec<Binding> = Vec::new();
        let mut functions: HashMap<String, u32> = HashMap::new();
        let mut bind = |call: &CodeCall| -> u32 {
            let key = format!("{}{}", call.function, serde_json::Value::Object(call.with.clone()));
            *functions.entry(key).or_insert_with(|| {
                bindings.push(library.bind(&call.function, &call.with, &|name| ids.get(name).copied()).expect("checked above"));
                bindings.len() as u32 - 1
            })
        };
        let hook = |hook: &crate::CodeHook, bind: &mut dyn FnMut(&CodeCall) -> u32| Hook {
            function: bind(&hook.code),
            triggers: Triggers { front: hook.at.contains(&TriggerPoint::Front), booking_point: hook.at.contains(&TriggerPoint::BookingPoint), rear: hook.at.contains(&TriggerPoint::Rear) },
        };
        for index in 0..self.instances.len() {
            let resolved = self.resolve(index).expect("checked: every instance's chain of classes is sound");
            let attributes = &resolved.attributes;
            let mut hooks = ObjectHooks {
                on_entry: attributes.on_entry.as_ref().map(|on_entry| hook(on_entry, &mut bind)),
                on_exit: attributes.on_exit.as_ref().map(|on_exit| hook(on_exit, &mut bind)),
                ..ObjectHooks::default()
            };
            if let Some(DistributionStrategy::Code(code)) = &attributes.distribution {
                hooks.distribution = Some(bind(&code.code));
                hooks.distribution_on_entry = code.asked_on_entry;
            }
            if hooks != ObjectHooks::default() {
                description.set_object_hooks(index as u32, hooks);
            }
        }
        for (_, moment, call) in self.hooks.each() {
            if let Some(call) = call {
                let function = bind(call);
                description.set_lifecycle_hook(moment, Some(function));
            }
        }
        let options = Options { mu_classes, ..options };
        let code = Code::new(library, bindings, &description, options).map_err(|message| vec![Problem { pointer: String::new(), name: None, message }])?;
        description.set_hooks(Box::new(code));
        Ok(description)
    }

    /// The model built with its code, loaded from `<models_folder>/code/`: every problem with the
    /// code, or the description.
    pub fn to_description_in(&self, models_folder: &Path, options: Options) -> Result<ModelDescription, Vec<Problem>> {
        let (library, problems) = self.load_code(models_folder);
        if !problems.is_empty() {
            return Err(problems);
        }
        self.to_description_with_code(Arc::new(library), options)
    }
}

/// The calls of code an owner's own attributes make.
fn push_calls<'m>(calls: &mut Vec<Named<'m>>, attributes: &'m crate::Attributes, owner: String, name: &str) {
    for (key, hook) in [("on_entry", &attributes.on_entry), ("on_exit", &attributes.on_exit)] {
        if let Some(hook) = hook {
            calls.push(Named { call: &hook.code, pointer: format!("{owner}/attributes/{key}/code"), name: Some(name.to_string()) });
        }
    }
    if let Some(DistributionStrategy::Code(code)) = &attributes.distribution {
        calls.push(Named { call: &code.code, pointer: format!("{owner}/attributes/distribution/code"), name: Some(name.to_string()) });
    }
}

