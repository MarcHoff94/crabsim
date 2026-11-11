//! Changing a model through operations: `edit_model` applies a batch of typed changes to a model
//! file, all or nothing, and `get_attributes` shows an instance's or class's values and where
//! each comes from. See `docs/design/operation-catalogue.md`.
//!
//! A batch applies to the file's JSON in canonical form, and only its result is checked, by the
//! model file's own rules through `crabsim_model::load`. So a batch may pass through invalid
//! states, values are written and checked exactly as in a file, and a problem's pointer points
//! into the file the batch would have produced.

use crate::error::OperationError;
use crate::model_files::read_file;
use crate::{entry, Catalogue, Effect, Entry, Operation};
use crabsim_model::{Attribute, Origin, BUILT_IN_CLASSES};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

pub(crate) fn entries() -> Vec<Entry> {
    vec![entry::<EditModel>(), entry::<GetAttributes>()]
}

/// The SHA-256 of a file's text, in hex. For a file crabsim saved, which is canonical, it equals
/// the `model_hash` the results store keeps for runs of it.
pub(crate) fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The SHA-256 of a valid model in canonical form, as runs record it, or none for an invalid one.
pub(crate) fn model_hash(text: &str) -> Option<String> {
    crabsim_model::load(text).ok().map(|model| hash(&model.save()))
}

/// A position in metres, the centre of the object's footprint.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    /// 0 if left out.
    pub z: Option<f64>,
}

impl Position {
    fn to_json(self) -> Value {
        match self.z {
            Some(z) => json!({ "x": self.x, "y": self.y, "z": z }),
            None => json!({ "x": self.x, "y": self.y }),
        }
    }
}

/// What an attribute is set on: one instance, or a class and everything that inherits from it.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    /// An instance, by name, such as `{"instance": "Mill 1"}`.
    Instance(String),
    /// A class, by name, such as `{"class": "CNC mill"}`. A built-in class's name, such as
    /// `{"class": "Station"}`, means this model's own values for it, which every class and
    /// instance built on it inherits: setting one makes them, and restoring the last one removes
    /// them, so the built-in's defaults hold again.
    Class(String),
}

/// One change to a model. Names are instance or class names, and attribute values are written
/// exactly as in a model file, such as `"0:45"` for a time.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "change", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    /// Adds an instance at the end of the instances, so runs stay paired with runs before.
    AddInstance {
        name: String,
        class: String,
        position: Option<Position>,
        /// The instances it passes MUs to, connected in this order of exits, each as the last
        /// entrance of its successor.
        successors: Option<Vec<String>>,
        /// Values set on this instance, as in a model file.
        attributes: Option<Map<String, Value>>,
    },
    /// Removes an instance and every connection to and from it. The instances after it move up,
    /// so runs before and after aren't paired any more.
    RemoveInstance { name: String },
    /// Renames an instance and every connection naming it.
    RenameInstance { name: String, to: String },
    /// Moves an instance to a new position.
    MoveInstance { name: String, position: Position },
    /// Makes an instance one of another class. The values set on it stay.
    SetClass { name: String, class: String },
    /// Adds a copy of an instance at the end, with its class and the values set on it, at
    /// `position` or the original's, with no connections.
    DuplicateInstance { name: String, to: String, position: Option<Position> },
    /// Connects `from` to `to`: as `from`'s exit `exit` (1 first, the order MUs are offered on) or
    /// its last, and as `to`'s entrance `entrance` or its last. The other connections at both ends
    /// move up to make room.
    Connect {
        from: String,
        to: String,
        exit: Option<u64>,
        entrance: Option<u64>,
        /// What the connection is, such as "via the lift".
        description: Option<String>,
    },
    /// Removes the connection from `from` to `to`. The other connections at both ends close up.
    Disconnect { from: String, to: String },
    /// Moves the connection from `from` to `to` to exit `exit` of `from` or entrance `entrance` of
    /// `to`, the others moving to make room, or sets its description, which `""` removes.
    SetConnection { from: String, to: String, exit: Option<u64>, entrance: Option<u64>, description: Option<String> },
    /// Adds a class derived from `parent`, a built-in or another class.
    AddClass {
        name: String,
        parent: String,
        /// Values the class changes from its parent, as in a model file.
        attributes: Option<Map<String, Value>>,
    },
    /// Removes a class. Refused while classes derive from it or instances are of it, listing
    /// them, unless `delete_dependents` is true, which removes them all with their connections.
    /// On a built-in class's name it removes only this model's own values for it, so the
    /// built-in's defaults hold again, and nothing depending on it goes.
    RemoveClass {
        name: String,
        #[serde(default)]
        delete_dependents: bool,
    },
    /// Renames a class and every reference to it.
    RenameClass { name: String, to: String },
    /// Makes a class derive from another parent.
    SetParent { name: String, parent: String },
    /// Adds a copy of a class, with its parent and values, under a new name, not linked to it.
    /// Its derived classes and instances aren't copied.
    DuplicateClass { name: String, to: String },
    /// Sets an attribute on an instance or class, which cuts its inheritance of it.
    SetAttribute { target: Target, attribute: String, value: Value },
    /// Removes an attribute set on an instance or class, so it inherits it again.
    RestoreInheritance { target: Target, attribute: String },
    /// Sets the model's own description, or removes it if left out.
    SetModelDescription { description: Option<String> },
    /// For the user only: gives an instance another stream, which ends pairing with every earlier
    /// run of it. Not advised: crabsim gives each instance its stream when it's placed.
    SetStream { name: String, stream: u64 },
}

impl Change {
    fn kind(&self) -> &'static str {
        match self {
            Change::AddInstance { .. } => "add_instance",
            Change::RemoveInstance { .. } => "remove_instance",
            Change::RenameInstance { .. } => "rename_instance",
            Change::MoveInstance { .. } => "move_instance",
            Change::SetClass { .. } => "set_class",
            Change::DuplicateInstance { .. } => "duplicate_instance",
            Change::Connect { .. } => "connect",
            Change::Disconnect { .. } => "disconnect",
            Change::SetConnection { .. } => "set_connection",
            Change::AddClass { .. } => "add_class",
            Change::RemoveClass { .. } => "remove_class",
            Change::RenameClass { .. } => "rename_class",
            Change::SetParent { .. } => "set_parent",
            Change::DuplicateClass { .. } => "duplicate_class",
            Change::SetAttribute { .. } => "set_attribute",
            Change::RestoreInheritance { .. } => "restore_inheritance",
            Change::SetModelDescription { .. } => "set_model_description",
            Change::SetStream { .. } => "set_stream",
        }
    }
}

/// A connection, from an instance to its successor.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Connection {
    pub from: String,
    pub to: String,
}

/// What a batch removed beyond what it named.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct AlsoRemoved {
    /// Instances removed with a class.
    pub instances: Vec<String>,
    /// Classes removed with a class they derive from.
    pub classes: Vec<String>,
    /// Connections removed with an instance.
    pub connections: Vec<Connection>,
}

/// The model's JSON while a batch applies to it.
struct Draft {
    model: Value,
    also: AlsoRemoved,
    /// The changes come from the user, who alone may set a stream.
    user: bool,
}

type Refusal = String;

impl Draft {
    fn instances(&mut self) -> &mut Vec<Value> {
        self.model["instances"].as_array_mut().expect("a loaded model has instances")
    }

    fn classes(&mut self) -> &mut Vec<Value> {
        let fields = self.model.as_object_mut().expect("a model is an object");
        fields.entry("classes").or_insert_with(|| json!([])).as_array_mut().expect("classes are a list")
    }

    fn instance(&mut self, name: &str) -> Result<usize, Refusal> {
        self.instances().iter().position(|instance| instance["name"] == name).ok_or_else(|| format!("no instance is called `{name}`"))
    }

    /// The class called `name`, which mustn't be a built-in class: those keep their names and
    /// parents, and only their values change, through `Target::Class`.
    fn class(&mut self, name: &str) -> Result<usize, Refusal> {
        if BUILT_IN_CLASSES.contains(&name) {
            return Err(format!("`{name}` is a built-in class, which keeps its name and parent: only its values can change for this model"));
        }
        self.classes().iter().position(|class| class["name"] == name).ok_or_else(|| format!("no class is called `{name}`"))
    }

    /// This model's own values for the built-in class `name`, if it has set any.
    fn refinement(&mut self, name: &str) -> Option<usize> {
        self.classes().iter().position(|class| class["name"] == name)
    }

    fn class_exists(&mut self, name: &str) -> Result<(), Refusal> {
        if BUILT_IN_CLASSES.contains(&name) || self.classes().iter().any(|class| class["name"] == name) {
            Ok(())
        } else {
            Err(format!("no class is called `{name}`"))
        }
    }

    fn new_instance_name(&mut self, name: &str) -> Result<(), Refusal> {
        if self.instances().iter().any(|instance| instance["name"] == name) {
            return Err(format!("an instance is already called `{name}`"));
        }
        Ok(())
    }

    fn new_class_name(&mut self, name: &str) -> Result<(), Refusal> {
        if BUILT_IN_CLASSES.contains(&name) {
            return Err(format!("`{name}` is a built-in class"));
        }
        if self.classes().iter().any(|class| class["name"] == name) {
            return Err(format!("a class is already called `{name}`"));
        }
        Ok(())
    }

    fn connections(&mut self) -> &mut Vec<Value> {
        let fields = self.model.as_object_mut().expect("a model is an object");
        fields.entry("connections").or_insert_with(|| json!([])).as_array_mut().expect("connections are a list")
    }

    /// The connection from `from` to `to`, by its place in the list.
    fn connection(&mut self, from: &str, to: &str) -> Result<usize, Refusal> {
        self.connections()
            .iter()
            .position(|connection| connection["from"] == from && connection["to"] == to)
            .ok_or_else(|| format!("`{from}` isn't connected to `{to}`"))
    }

    /// The connections whose `end` (`from` or `to`) is `name`, by place in the list, in the order
    /// of `field` (`exit` or `entrance`), leaving out `except`.
    fn ordered(&mut self, end: &str, name: &str, field: &str, except: Option<usize>) -> Vec<usize> {
        let connections = self.connections();
        let mut order: Vec<usize> =
            (0..connections.len()).filter(|&index| Some(index) != except && connections[index][end] == name).collect();
        order.sort_by_key(|&index| connections[index][field].as_u64().unwrap_or(u64::MAX));
        order
    }

    /// Numbers these connections' `field` 1 to n, in this order.
    fn number(&mut self, order: &[usize], field: &str) {
        for (place, &index) in order.iter().enumerate() {
            self.connections()[index][field] = json!(place + 1);
        }
    }

    /// Checks that `place` can be the `field` of a connection joining the others whose `end` is
    /// `name`, `moving` being the connection itself if it is already there.
    fn check_place(&mut self, end: &str, name: &str, field: &str, place: Option<u64>, moving: Option<usize>) -> Result<(), Refusal> {
        let count = self.ordered(end, name, field, moving).len() as u64 + 1;
        match place {
            Some(place) if place < 1 || place > count => {
                let way = if end == "from" { "out" } else { "in" };
                let others = count - 1;
                let plural = if others == 1 { "" } else { "s" };
                let range = if count == 1 { "only be 1".to_string() } else { format!("be 1 to {count}") };
                Err(format!("`{name}` has {others} other connection{plural} {way}, so its {field} can {range}, not {place}"))
            }
            _ => Ok(()),
        }
    }

    /// Puts the connection at `index` at `place` among those whose `end` is `name`, or last, and
    /// numbers them all again. The place must have passed `check_place`.
    fn place(&mut self, index: usize, end: &str, name: &str, field: &str, place: Option<u64>) {
        let mut order = self.ordered(end, name, field, Some(index));
        let at = place.map_or(order.len(), |place| place as usize - 1);
        order.insert(at, index);
        self.number(&order, field);
    }

    /// Connects `from` to `to`, checking both names and both places first, so a refusal changes
    /// nothing.
    fn connect(&mut self, from: &str, to: &str, exit: Option<u64>, entrance: Option<u64>, description: Option<String>) -> Result<(), Refusal> {
        self.instance(from)?;
        self.instance(to)?;
        if self.connection(from, to).is_ok() {
            return Err(format!("`{from}` is already connected to `{to}`"));
        }
        self.check_place("from", from, "exit", exit, None)?;
        self.check_place("to", to, "entrance", entrance, None)?;
        let mut connection = json!({ "from": from, "to": to, "exit": 0, "entrance": 0 });
        if let Some(description) = description.filter(|description| !description.is_empty()) {
            connection["description"] = json!(description);
        }
        self.connections().push(connection);
        let index = self.connections().len() - 1;
        self.place(index, "from", from, "exit", exit);
        self.place(index, "to", to, "entrance", entrance);
        Ok(())
    }

    /// Removes the connection at `index`, closing up the exits and entrances at both its ends.
    fn disconnect(&mut self, index: usize) -> Connection {
        let from = self.connections()[index]["from"].as_str().unwrap_or_default().to_string();
        let to = self.connections()[index]["to"].as_str().unwrap_or_default().to_string();
        let exits = self.ordered("from", &from, "exit", Some(index));
        self.number(&exits, "exit");
        let entrances = self.ordered("to", &to, "entrance", Some(index));
        self.number(&entrances, "entrance");
        self.connections().remove(index);
        Connection { from, to }
    }

    /// The attributes of `target`, made if there are none yet.
    fn attributes(&mut self, target: &Target) -> Result<(&mut Map<String, Value>, String), Refusal> {
        let (owner, what) = match target {
            Target::Instance(name) => {
                let index = self.instance(name)?;
                (&mut self.instances()[index], format!("instance `{name}`"))
            }
            Target::Class(name) if BUILT_IN_CLASSES.contains(&name.as_str()) => {
                let index = match self.refinement(name) {
                    Some(index) => index,
                    None => {
                        self.classes().push(json!({ "name": name }));
                        self.classes().len() - 1
                    }
                };
                (&mut self.classes()[index], format!("this model's class `{name}`"))
            }
            Target::Class(name) => {
                let index = self.class(name)?;
                (&mut self.classes()[index], format!("class `{name}`"))
            }
        };
        let fields = owner.as_object_mut().expect("an owner is an object");
        let attributes = fields.entry("attributes").or_insert_with(|| json!({})).as_object_mut().ok_or("its attributes aren't an object")?;
        Ok((attributes, what))
    }

    /// Removes the instance at `index` and every connection to and from it, in the order of the
    /// list, closing up the exits and entrances of the instances they joined it to.
    fn remove_instance(&mut self, index: usize) {
        let removed = self.instances().remove(index);
        let name = removed["name"].as_str().unwrap_or_default().to_string();
        while let Some(at) = self.connections().iter().position(|connection| connection["from"] == name || connection["to"] == name) {
            let connection = self.disconnect(at);
            self.also.connections.push(connection);
        }
    }

    /// The classes deriving from `name` through any depth, in file order.
    fn derived_from(&mut self, name: &str) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        loop {
            let more: Vec<String> = self
                .classes()
                .iter()
                .filter(|class| {
                    let parent = class["parent"].as_str().unwrap_or_default();
                    (parent == name || found.iter().any(|f| f == parent)) && !found.iter().any(|f| *f == class["name"])
                })
                .map(|class| class["name"].as_str().unwrap_or_default().to_string())
                .collect();
            if more.is_empty() {
                break;
            }
            found.extend(more);
        }
        let order: Vec<String> = self.classes().iter().map(|class| class["name"].as_str().unwrap_or_default().to_string()).collect();
        found.sort_by_key(|name| order.iter().position(|o| o == name));
        found
    }

    fn apply(&mut self, change: Change) -> Result<(), Refusal> {
        match change {
            Change::AddInstance { name, class, position, successors, attributes } => {
                self.new_instance_name(&name)?;
                self.class_exists(&class)?;
                let successors = successors.unwrap_or_default();
                for successor in &successors {
                    if *successor != name {
                        self.instance(successor)?;
                    }
                }
                let mut instance = json!({ "name": name, "class": class });
                if let Some(position) = position {
                    instance["position"] = position.to_json();
                }
                if let Some(attributes) = attributes {
                    instance["attributes"] = Value::Object(attributes);
                }
                self.instances().push(instance);
                // Repeats and a connection to itself are left for the model's own check to report.
                for successor in successors {
                    let mut connection = json!({ "from": name, "to": successor, "exit": 0, "entrance": 0 });
                    connection["exit"] = json!(self.ordered("from", &name, "exit", None).len() + 1);
                    connection["entrance"] = json!(self.ordered("to", &successor, "entrance", None).len() + 1);
                    self.connections().push(connection);
                }
            }
            Change::RemoveInstance { name } => {
                let index = self.instance(&name)?;
                self.remove_instance(index);
            }
            Change::RenameInstance { name, to } => {
                let index = self.instance(&name)?;
                self.new_instance_name(&to)?;
                self.instances()[index]["name"] = json!(to);
                for connection in self.connections().iter_mut() {
                    for end in ["from", "to"] {
                        if connection[end] == name.as_str() {
                            connection[end] = json!(to);
                        }
                    }
                }
            }
            Change::MoveInstance { name, position } => {
                let index = self.instance(&name)?;
                self.instances()[index]["position"] = position.to_json();
            }
            Change::SetClass { name, class } => {
                let index = self.instance(&name)?;
                self.class_exists(&class)?;
                self.instances()[index]["class"] = json!(class);
            }
            Change::DuplicateInstance { name, to, position } => {
                let index = self.instance(&name)?;
                self.new_instance_name(&to)?;
                let mut copy = self.instances()[index].clone();
                let fields = copy.as_object_mut().expect("an instance is an object");
                fields.insert("name".into(), json!(to));
                // A copy is a new instance, so it gets a stream of its own when the model is read.
                fields.remove("stream");
                if let Some(position) = position {
                    fields.insert("position".into(), position.to_json());
                }
                self.instances().push(copy);
            }
            Change::Connect { from, to, exit, entrance, description } => {
                self.connect(&from, &to, exit, entrance, description)?;
            }
            Change::Disconnect { from, to } => {
                self.instance(&from)?;
                self.instance(&to)?;
                let index = self.connection(&from, &to)?;
                self.disconnect(index);
            }
            Change::SetConnection { from, to, exit, entrance, description } => {
                let index = self.connection(&from, &to)?;
                self.check_place("from", &from, "exit", exit, Some(index))?;
                self.check_place("to", &to, "entrance", entrance, Some(index))?;
                if exit.is_some() {
                    self.place(index, "from", &from, "exit", exit);
                }
                if entrance.is_some() {
                    self.place(index, "to", &to, "entrance", entrance);
                }
                let fields = self.connections()[index].as_object_mut().expect("a connection is an object");
                match description {
                    Some(text) if text.is_empty() => {
                        fields.remove("description");
                    }
                    Some(text) => {
                        fields.insert("description".into(), json!(text));
                    }
                    None => {}
                }
            }
            Change::AddClass { name, parent, attributes } => {
                self.new_class_name(&name)?;
                self.class_exists(&parent)?;
                let mut class = json!({ "name": name, "parent": parent });
                if let Some(attributes) = attributes {
                    class["attributes"] = Value::Object(attributes);
                }
                self.classes().push(class);
            }
            Change::RemoveClass { name, .. } if BUILT_IN_CLASSES.contains(&name.as_str()) => {
                let index = self.refinement(&name).ok_or_else(|| format!("this model sets nothing on the built-in class `{name}`, so there's nothing to remove"))?;
                self.classes().remove(index);
            }
            Change::RemoveClass { name, delete_dependents } => {
                self.class(&name)?;
                let derived = self.derived_from(&name);
                let mut family = derived.clone();
                family.push(name.clone());
                let instances: Vec<String> = self
                    .instances()
                    .iter()
                    .filter(|instance| family.iter().any(|class| instance["class"] == class.as_str()))
                    .map(|instance| instance["name"].as_str().unwrap_or_default().to_string())
                    .collect();
                if !delete_dependents && !(derived.is_empty() && instances.is_empty()) {
                    let list = |names: &[String]| names.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ");
                    let mut what = Vec::new();
                    if !derived.is_empty() {
                        what.push(format!("the classes {} derive from it", list(&derived)));
                    }
                    if !instances.is_empty() {
                        what.push(format!("the instances {} are of it or of a class derived from it, with their connections", list(&instances)));
                    }
                    return Err(format!(
                        "{} depend on `{name}`: removing it would remove them too. Give `delete_dependents: true` to remove them all, or change them first",
                        what.join(", and ")
                    ));
                }
                for instance in &instances {
                    let at = self.instance(instance)?;
                    self.remove_instance(at);
                }
                self.also.instances.extend(instances);
                self.classes().retain(|class| !family.iter().any(|name| class["name"] == name.as_str()));
                self.also.classes.extend(derived);
            }
            Change::RenameClass { name, to } => {
                let index = self.class(&name)?;
                self.new_class_name(&to)?;
                self.classes()[index]["name"] = json!(to);
                for class in self.classes().iter_mut() {
                    if class["parent"] == name.as_str() {
                        class["parent"] = json!(to);
                    }
                }
                for instance in self.instances().iter_mut() {
                    if instance["class"] == name.as_str() {
                        instance["class"] = json!(to);
                    }
                }
            }
            Change::SetParent { name, parent } => {
                let index = self.class(&name)?;
                self.class_exists(&parent)?;
                self.classes()[index]["parent"] = json!(parent);
            }
            Change::DuplicateClass { name, to } => {
                let index = self.class(&name)?;
                self.new_class_name(&to)?;
                let mut copy = self.classes()[index].clone();
                copy["name"] = json!(to);
                self.classes().push(copy);
            }
            Change::SetAttribute { target, attribute, value } => {
                let (attributes, _) = self.attributes(&target)?;
                attributes.insert(attribute, value);
            }
            Change::RestoreInheritance { target, attribute } => {
                if let Target::Class(name) = &target
                    && BUILT_IN_CLASSES.contains(&name.as_str())
                    && self.refinement(name).is_none()
                {
                    return Err(format!("`{attribute}` isn't set on this model's class `{name}`, so it has the built-in default"));
                }
                let (attributes, what) = self.attributes(&target)?;
                if attributes.remove(&attribute).is_none() {
                    return Err(format!("`{attribute}` isn't set on {what}, so it already inherits it"));
                }
                let nothing_left = attributes.is_empty();
                // With nothing left set, this model's values for a built-in class go.
                if let Target::Class(name) = &target
                    && BUILT_IN_CLASSES.contains(&name.as_str())
                    && nothing_left
                    && let Some(index) = self.refinement(name)
                {
                    self.classes().remove(index);
                }
            }
            Change::SetStream { name, stream } => {
                if !self.user {
                    return Err("set_stream is only for the user: crabsim gives each instance its stream when it's placed, and changing it ends pairing with earlier runs".to_string());
                }
                let index = self.instance(&name)?;
                self.instances()[index]["stream"] = json!(stream);
            }
            Change::SetModelDescription { description } => {
                let fields = self.model.as_object_mut().expect("a model is an object");
                match description {
                    Some(description) => fields.insert("description".into(), json!(description)),
                    None => fields.remove("description"),
                };
            }
        }
        Ok(())
    }
}

// --- edit_model ---

pub struct EditModel;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditModelInput {
    /// The model file to change, in the models folder, such as `line.json`. It must be valid:
    /// fix an invalid one with write_model.
    pub path: String,
    /// The changes, applied in order, all or nothing. Each names its kind in `change`.
    pub changes: Vec<Change>,
    /// The hash of the file as you read it, from read_model or get_attributes: the batch is
    /// refused as a conflict if the file has changed since.
    pub expected_hash: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct EditModelOutput {
    pub path: String,
    /// SHA-256 of the file as saved, for the next `expected_hash`.
    pub file_hash: String,
    /// SHA-256 of the model in canonical form, as runs of it will record it: equal to
    /// `file_hash`, since crabsim saved it.
    pub model_hash: String,
    /// How many changes were applied.
    pub changes: usize,
    /// What the changes removed beyond what they named: connections with an instance, and
    /// instances and classes with a class.
    pub also_removed: AlsoRemoved,
}

impl Operation for EditModel {
    const NAME: &'static str = "edit_model";
    const DESCRIPTION: &'static str = "Changes a valid model file in the models folder with a batch of changes, applied in order, all or nothing, then saves it in canonical form. Each change is an object naming its kind in `change`: add_instance, remove_instance, rename_instance, move_instance, set_class, duplicate_instance, connect, disconnect, set_connection, add_class, remove_class, rename_class, set_parent, duplicate_class, set_attribute, restore_inheritance or set_model_description, and for the user set_stream. Values are written as in a model file, such as \"0:45\" for a time (model_file_format gives the format). Only the result is checked, so a batch may pass through invalid states, such as adding a Source and then setting its interval. A change that can't apply, such as one naming an unknown instance, refuses the batch with invalid_input naming its place, such as changes/2, along with every other that can't and the problems of the model the rest would make, so one round fixes them all. A result with problems refuses it with invalid_model and every problem, each pointing into the file the batch would have produced. Either way the file is unchanged. Connections have an exit, their place in the order their object offers MUs on, and an entrance, their place among those into the object they lead to: connect, disconnect, set_connection and removing an instance number them again at both ends, so give a place only to choose one. Give `expected_hash`, the file_hash read_model or get_attributes gave, to have the batch refused as a conflict if the file changed since you read it. An agent needs its claim on the file first (claim_model), and no file changed outside crabsim is changed until adopt_model. On success it gives the file's new file_hash, and what else was removed, such as the connections of a removed instance. Every instance keeps its stream, which gives its random numbers, whatever else changes, so runs of the model before and after an edit stay paired for every instance they share. New and duplicated instances get a stream of their own and go at the end.";
    const EFFECT: Effect = Effect::Destructive;
    const CHANGES_MODELS: bool = true;
    type Input = EditModelInput;
    type Output = EditModelOutput;

    fn run(catalogue: &Catalogue, input: EditModelInput) -> Result<EditModelOutput, OperationError> {
        let mut editing = crate::editing::before_change(catalogue, &input.path, input.expected_hash.as_deref())?;
        let (file, text) = read_file(catalogue, &input.path)?;
        let model = crabsim_model::load(&text).map_err(|problems| {
            let mut error = OperationError::invalid_model(&format!("`{}`", input.path), &problems);
            error.message = format!("{}, so edit_model can't change it: fix it with write_model first", error.message);
            error
        })?;
        let mut draft = Draft {
            model: serde_json::from_str(&model.save()).expect("a saved model is JSON"),
            also: AlsoRemoved::default(),
            user: catalogue.source().is_user(),
        };
        let count = input.changes.len();
        // Every change that can't apply, after which the rest still apply, so the result's own
        // problems can be reported along with them in one go.
        let mut refused = Vec::new();
        for (place, change) in input.changes.into_iter().enumerate() {
            let kind = change.kind();
            if let Err(why) = draft.apply(change) {
                refused.push(format!("changes/{place} ({kind}): {why}"));
            }
        }
        if !refused.is_empty() {
            let mut error = OperationError::invalid_input(format!("{}. Nothing was changed", refused.join("; ")));
            if let Err(problems) = crabsim_model::load(&draft.model.to_string()) {
                error.problems = problems.iter().map(crate::ModelProblem::from).collect();
                error.message += ". The problems are those of the model the other changes would make";
            }
            return Err(error);
        }
        let edited = crabsim_model::load(&draft.model.to_string())
            .map_err(|problems| OperationError::invalid_model("the model these changes would make", &problems))?
            .save();
        std::fs::write(&file, &edited).map_err(|error| OperationError::run_error(format!("`{}` can't be written: {error}", input.path)))?;
        editing.record(&edited);
        Ok(EditModelOutput {
            path: input.path,
            file_hash: hash(&edited),
            model_hash: hash(&edited),
            changes: count,
            also_removed: draft.also,
        })
    }
}

// --- get_attributes ---

pub struct GetAttributes;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetAttributesInput {
    /// The model file, in the models folder, such as `line.json`.
    pub path: String,
    /// The instance or class, such as `{"instance": "Mill 1"}` or `{"class": "CNC mill"}`. A
    /// built-in class, such as `{"class": "Station"}`, gives the defaults.
    pub target: Target,
}

/// Where a value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComesFrom {
    /// Set on the instance itself, which cuts its inheritance.
    Instance,
    /// From the class named in `class`, the nearest in the chain that sets it.
    Class,
    /// The built-in class's default.
    BuiltIn,
    /// Set nowhere, and with no default.
    NotSet,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AttributeValue {
    pub attribute: String,
    /// As a model file writes it. Left out when not set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    pub from: ComesFrom,
    /// The class it comes from, when `from` is class.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct GetAttributesOutput {
    pub path: String,
    /// SHA-256 of the file's bytes, for edit_model's `expected_hash`.
    pub file_hash: String,
    /// SHA-256 of the model in canonical form, as runs record it.
    pub model_hash: String,
    /// The file was changed outside crabsim, or crabsim never saved it: adopt_model before
    /// changing it.
    pub changed_outside: bool,
    pub built_in_class: String,
    /// Its classes, nearest first, ending with the built-in class.
    pub chain: Vec<String>,
    /// Every attribute its built-in class has, in a fixed order.
    pub attributes: Vec<AttributeValue>,
    /// An instance's stream, which with a run's seed gives its random numbers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<u64>,
    /// An instance's own position, if it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Value>,
    /// An instance's successors, by exit: the order it offers MUs on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub successors: Option<Vec<String>>,
    /// An instance's predecessors, by entrance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predecessors: Option<Vec<String>>,
}

impl Operation for GetAttributes {
    const NAME: &'static str = "get_attributes";
    const DESCRIPTION: &'static str = "Shows every attribute of an instance or class in a model file, with its value as a model file writes it and where it comes from: `from` is instance (set on it), class (inherited, with the class named), built_in (the built-in default) or not_set. It also gives the chain of classes, nearest first, an instance's stream, position, successors (by exit) and predecessors (by entrance), the file's file_hash, and the model_hash runs of it record. Use it to see why an object behaves as it does before changing it with edit_model.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = GetAttributesInput;
    type Output = GetAttributesOutput;

    fn run(catalogue: &Catalogue, input: GetAttributesInput) -> Result<GetAttributesOutput, OperationError> {
        let (_, text) = read_file(catalogue, &input.path)?;
        let model = crabsim_model::load(&text).map_err(|problems| OperationError::invalid_model(&format!("`{}`", input.path), &problems))?;
        let (resolved, class, instance) = match &input.target {
            Target::Instance(name) => {
                let index = model
                    .instances
                    .iter()
                    .position(|instance| &instance.name == name)
                    .ok_or_else(|| OperationError::not_found(format!("`{}` has no instance called `{name}`", input.path)))?;
                (model.resolve(index), model.instances[index].class.clone(), Some(&model.instances[index]))
            }
            Target::Class(name) => (model.resolve_class(name), name.clone(), None),
        };
        let resolved = resolved.ok_or_else(|| OperationError::not_found(format!("`{}` has no class called `{class}`", input.path)))?;
        let (chain, built_in) = model.chain(&class).expect("resolved, so the chain is sound");
        let mut chain: Vec<String> = chain.iter().map(|class| class.name.clone()).collect();
        if let Target::Class(name) = &input.target {
            chain.retain(|class| class != name);
        }
        // The model's own values for the built-in class, if any, are last in the chain already.
        if chain.last().map(String::as_str) != Some(built_in) {
            chain.push(built_in.to_string());
        }
        let values = resolved.attributes.to_json();
        let attributes = Attribute::ALL
            .into_iter()
            .filter(|attribute| attribute.applies_to(built_in))
            .map(|attribute| {
                let value = values.get(attribute.name()).cloned();
                let (from, class) = match (resolved.origin(attribute), &value) {
                    (Origin::Instance, _) => (ComesFrom::Instance, None),
                    (Origin::Class(class), _) => (ComesFrom::Class, Some(class.clone())),
                    (Origin::BuiltIn, Some(_)) => (ComesFrom::BuiltIn, None),
                    (Origin::BuiltIn, None) => (ComesFrom::NotSet, None),
                };
                AttributeValue { attribute: attribute.name().to_string(), value, from, class }
            })
            .collect();
        let changed_outside = crate::editing::changed_outside(catalogue, &input.path, &text)?;
        Ok(GetAttributesOutput {
            path: input.path,
            file_hash: hash(&text),
            model_hash: hash(&model.save()),
            changed_outside,
            built_in_class: built_in.to_string(),
            chain,
            attributes,
            position: instance.and_then(|instance| instance.position).map(|position| {
                if position.z == 0.0 { json!({ "x": position.x, "y": position.y }) } else { json!({ "x": position.x, "y": position.y, "z": position.z }) }
            }),
            successors: instance.map(|instance| model.successors(&instance.name).into_iter().map(str::to_owned).collect()),
            predecessors: instance.map(|instance| model.predecessors(&instance.name).into_iter().map(str::to_owned).collect()),
            stream: instance.and_then(|instance| instance.stream),
        })
    }
}
