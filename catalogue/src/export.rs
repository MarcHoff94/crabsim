//! `export_model`: a model as a graph file for other tools, a Cypher script, Neo4j's CSV import or
//! GraphML, written into `exports/` in the models folder (spec issue 56). The graph is the same in
//! every format: instances labelled with their built-in class and carrying their resolved values,
//! classes with the values they set, and `FEEDS`, `IS_A` and `DERIVES_FROM` connections. An export
//! is derived and never edited, so exporting again overwrites it.

use crate::error::OperationError;
use crate::folder::EXPORTS_FOLDER;
use crate::graph::model_text;
use crate::{entry, Catalogue, Effect, Entry, Operation};
use crabsim_model::{Attribute, Attributes, Model, RepairTime, BUILT_IN_CLASSES};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

pub(crate) fn entries() -> Vec<Entry> {
    vec![entry::<ExportModel>()]
}

// --- The graph, the same in every format ---

/// A property's value, typed, since every format but Cypher names the type.
#[derive(Debug, Clone)]
enum Value {
    Text(String),
    Long(u64),
    Double(f64),
    Bool(bool),
}

impl Value {
    /// The type as Neo4j's CSV import and GraphML name it.
    fn type_name(&self) -> &'static str {
        match self {
            Value::Text(_) => "string",
            Value::Long(_) => "long",
            Value::Double(_) => "double",
            Value::Bool(_) => "boolean",
        }
    }

    /// As CSV and GraphML write it.
    fn plain(&self) -> String {
        match self {
            Value::Text(text) => text.clone(),
            Value::Long(n) => n.to_string(),
            Value::Double(x) => x.to_string(),
            Value::Bool(b) => b.to_string(),
        }
    }

    /// As a Cypher literal, where a double always has a decimal point.
    fn cypher(&self) -> String {
        match self {
            Value::Text(text) => cypher_text(text),
            Value::Double(x) => format!("{x:?}"),
            other => other.plain(),
        }
    }
}

type Properties = Vec<(&'static str, Value)>;

/// A node: its labels and properties, `name` first.
struct Node {
    labels: Vec<&'static str>,
    properties: Properties,
}

impl Node {
    fn name(&self) -> &str {
        match &self.properties[0] {
            ("name", Value::Text(name)) => name,
            _ => unreachable!("a node's first property is its name"),
        }
    }
}

/// A model as the export's graph. Connections refer to instances and classes by place.
struct Graph {
    instances: Vec<Node>,
    classes: Vec<Node>,
    /// The connections, from an instance to a successor, each with its properties.
    feeds: Vec<(usize, usize, Vec<(&'static str, Value)>)>,
    /// From an instance to its class.
    is_a: Vec<(usize, usize)>,
    /// From a class to its parent.
    derives_from: Vec<(usize, usize)>,
}

fn seconds(time: crabsim::SimTime) -> Value {
    Value::Double(time.as_secs_f64())
}

/// The properties for the attributes in `attributes` that `has` lets through, with times in
/// seconds and failures and dimensions spread over plain properties.
fn attribute_properties(attributes: &Attributes, has: impl Fn(Attribute) -> bool, properties: &mut Properties) {
    if let Some(interval) = attributes.interval.filter(|_| has(Attribute::Interval)) {
        properties.push(("interval_s", seconds(interval)));
    }
    if let Some(time) = attributes.process_time.filter(|_| has(Attribute::ProcessTime)) {
        properties.push(("process_time_s", seconds(time)));
    }
    if let Some(failures) = attributes.failures.as_ref().filter(|_| has(Attribute::Failures)) {
        properties.push(("availability", Value::Double(failures.availability)));
        let (distribution, parameters): (&str, Properties) = match failures.repair_time {
            RepairTime::Constant(time) => ("constant", vec![("repair_time_s", seconds(time))]),
            RepairTime::Uniform { min, max } => ("uniform", vec![("repair_min_s", seconds(min)), ("repair_max_s", seconds(max))]),
            RepairTime::Normal { mean, std } => ("normal", vec![("repair_mean_s", seconds(mean)), ("repair_std_s", seconds(std))]),
            RepairTime::Erlang { mean, k } => ("erlang", vec![("repair_mean_s", seconds(mean)), ("repair_k", Value::Long(k))]),
        };
        properties.push(("repair_distribution", Value::Text(distribution.to_string())));
        properties.extend(parameters);
    }
    if let Some(mus) = attributes.starting_mus.filter(|_| has(Attribute::StartingMus)) {
        properties.push(("starting_mus", Value::Long(mus)));
    }
    if let Some(capacity) = attributes.capacity.filter(|_| has(Attribute::Capacity)) {
        properties.push(("capacity", Value::Long(capacity)));
    }
    if let Some(dimensions) = attributes.dimensions.filter(|_| has(Attribute::Dimensions)) {
        properties.push(("length", Value::Double(dimensions.length)));
        properties.push(("width", Value::Double(dimensions.width)));
        properties.push(("height", Value::Double(dimensions.height)));
        properties.push(("rotation_degrees", Value::Double(dimensions.rotation_degrees)));
    }
    if let Some(description) = attributes.description.as_ref().filter(|_| has(Attribute::Description)) {
        properties.push(("description", Value::Text(description.clone())));
    }
}

impl Graph {
    /// The graph of a valid model.
    fn of(model: &Model) -> Graph {
        let mut classes: Vec<Node> = BUILT_IN_CLASSES
            .iter()
            .map(|name| Node { labels: vec!["Class"], properties: vec![("name", Value::Text(name.to_string())), ("built_in", Value::Bool(true))] })
            .collect();
        // The model's own values for a built-in class go on the built-in's node.
        for class in &model.classes {
            if class.refines_built_in() {
                let at = BUILT_IN_CLASSES.iter().position(|name| *name == class.name).expect("a refinement is named like a built-in");
                attribute_properties(&class.attributes, |_| true, &mut classes[at].properties);
                continue;
            }
            let mut properties = vec![("name", Value::Text(class.name.clone())), ("built_in", Value::Bool(false))];
            attribute_properties(&class.attributes, |_| true, &mut properties);
            classes.push(Node { labels: vec!["Class"], properties });
        }
        let class_at = |name: &str| classes.iter().position(|class| class.name() == name).expect("a valid model's classes exist");
        let derives_from = model
            .classes
            .iter()
            .filter(|class| !class.refines_built_in())
            .map(|class| (class_at(&class.name), class_at(&class.parent)))
            .collect();

        let mut instances = Vec::new();
        let mut is_a = Vec::new();
        for (index, instance) in model.instances.iter().enumerate() {
            let resolved = model.resolve(index).expect("a valid model's instances resolve");
            let mut properties = vec![("name", Value::Text(instance.name.clone())), ("class", Value::Text(instance.class.clone()))];
            if let Some(stream) = instance.stream {
                properties.push(("stream", Value::Long(stream)));
            }
            if let Some(position) = instance.position {
                properties.push(("x", Value::Double(position.x)));
                properties.push(("y", Value::Double(position.y)));
                properties.push(("z", Value::Double(position.z)));
            }
            attribute_properties(&resolved.attributes, |attribute| attribute.applies_to(resolved.built_in), &mut properties);
            instances.push(Node { labels: vec!["Instance", resolved.built_in], properties });
            is_a.push((index, class_at(&instance.class)));
        }
        let place: std::collections::HashMap<&str, usize> = model.instances.iter().enumerate().map(|(i, instance)| (instance.name.as_str(), i)).collect();
        // Each instance's connections out in turn, by exit, as a saved file lists them.
        let feeds = model
            .instances
            .iter()
            .enumerate()
            .flat_map(|(from, instance)| model.connections_out(&instance.name).into_iter().map(move |connection| (from, connection)))
            .map(|(from, connection)| {
                let mut properties = vec![("exit", Value::Long(connection.exit)), ("entrance", Value::Long(connection.entrance))];
                if let Some(description) = &connection.description {
                    properties.push(("description", Value::Text(description.clone())));
                }
                (from, place[connection.to.as_str()], properties)
            })
            .collect();
        Graph { instances, classes, feeds, is_a, derives_from }
    }
}

/// Every property name the nodes have, in the order they first appear, with its type.
fn columns<'n>(nodes: impl IntoIterator<Item = &'n Node>) -> Vec<(&'static str, &'static str)> {
    let mut columns: Vec<(&'static str, &'static str)> = Vec::new();
    for node in nodes {
        for (key, value) in &node.properties {
            if !columns.iter().any(|(known, _)| known == key) {
                columns.push((key, value.type_name()));
            }
        }
    }
    columns
}

// --- Cypher ---

/// A Cypher string literal.
fn cypher_text(text: &str) -> String {
    let mut quoted = String::from("'");
    for c in text.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '\'' => quoted.push_str("\\'"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            c => quoted.push(c),
        }
    }
    quoted.push('\'');
    quoted
}

fn cypher(graph: &Graph, what: &str) -> String {
    let mut script = format!("// crabsim's export of {what}, as a graph: load it into an empty Neo4j database, such as with cypher-shell -f.\n");
    script += "CREATE INDEX crabsim_instance_name IF NOT EXISTS FOR (n:Instance) ON (n.name);\n";
    script += "CREATE INDEX crabsim_class_name IF NOT EXISTS FOR (n:Class) ON (n.name);\n";
    let node = |node: &Node| {
        let properties: Vec<String> = node.properties.iter().map(|(key, value)| format!("{key}: {}", value.cypher())).collect();
        format!("CREATE (:{} {{{}}});\n", node.labels.join(":"), properties.join(", "))
    };
    graph.classes.iter().for_each(|class| script += &node(class));
    graph.instances.iter().for_each(|instance| script += &node(instance));
    let edge = |script: &mut String, (from_label, from): (&str, &Node), (to_label, to): (&str, &Node), kind: &str| {
        let _ = writeln!(
            script,
            "MATCH (a:{from_label} {{name: {}}}), (b:{to_label} {{name: {}}}) CREATE (a)-[:{kind}]->(b);",
            cypher_text(from.name()),
            cypher_text(to.name())
        );
    };
    for (from, to, properties) in &graph.feeds {
        let properties: Vec<String> = properties.iter().map(|(key, value)| format!("{key}: {}", value.cypher())).collect();
        let kind = format!("FEEDS {{{}}}", properties.join(", "));
        edge(&mut script, ("Instance", &graph.instances[*from]), ("Instance", &graph.instances[*to]), &kind);
    }
    for &(instance, class) in &graph.is_a {
        edge(&mut script, ("Instance", &graph.instances[instance]), ("Class", &graph.classes[class]), "IS_A");
    }
    for &(class, parent) in &graph.derives_from {
        edge(&mut script, ("Class", &graph.classes[class]), ("Class", &graph.classes[parent]), "DERIVES_FROM");
    }
    script
}

// --- Neo4j's CSV import ---

/// A CSV field, quoted when it holds a comma, a quote or a line break.
fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) || text.starts_with(' ') || text.ends_with(' ') {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn csv_line(fields: impl IntoIterator<Item = String>) -> String {
    let fields: Vec<String> = fields.into_iter().map(|field| csv_field(&field)).collect();
    fields.join(",") + "\n"
}

/// One node file: the name as the id in `space`, the labels, then every property, each column
/// typed as Neo4j's import reads it, and empty where a node hasn't the property.
fn csv_nodes(nodes: &[Node], space: &str) -> String {
    let columns: Vec<_> = columns(nodes).into_iter().filter(|(key, _)| *key != "name").collect();
    let mut header = vec![format!("name:ID({space})"), ":LABEL".to_string()];
    header.extend(columns.iter().map(|(key, kind)| if *kind == "string" { key.to_string() } else { format!("{key}:{kind}") }));
    let mut text = csv_line(header);
    for node in nodes {
        let mut row = vec![node.name().to_string(), node.labels.join(";")];
        row.extend(columns.iter().map(|(key, _)| node.properties.iter().find(|(k, _)| k == key).map_or(String::new(), |(_, value)| value.plain())));
        text += &csv_line(row);
    }
    text
}

/// One relationship file: the two ends, then a column for each of `properties`, typed as Neo4j's
/// import reads it, and empty where an edge hasn't the property.
fn csv_edges<'g>(
    header: [&str; 2],
    kind: &str,
    properties: &[(&str, &str)],
    edges: impl Iterator<Item = (&'g str, &'g str, &'g [(&'static str, Value)])>,
) -> String {
    let mut columns: Vec<String> = header.iter().map(|column| column.to_string()).collect();
    columns.extend(properties.iter().map(|(key, kind)| if *kind == "string" { key.to_string() } else { format!("{key}:{kind}") }));
    columns.push(":TYPE".to_string());
    let mut text = csv_line(columns);
    for (from, to, values) in edges {
        let mut row = vec![from.to_string(), to.to_string()];
        row.extend(properties.iter().map(|(key, _)| values.iter().find(|(k, _)| k == key).map_or(String::new(), |(_, value)| value.plain())));
        row.push(kind.to_string());
        text += &csv_line(row);
    }
    text
}

/// A FEEDS edge's properties, with their types.
const FEEDS_PROPERTIES: [(&str, &str); 3] = [("exit", "long"), ("entrance", "long"), ("description", "string")];

/// The five files, by name.
fn neo4j_csv(graph: &Graph) -> Vec<(&'static str, String)> {
    let feeds = graph.feeds.iter().map(|(from, to, properties)| (graph.instances[*from].name(), graph.instances[*to].name(), properties.as_slice()));
    let is_a = graph.is_a.iter().map(|&(instance, class)| (graph.instances[instance].name(), graph.classes[class].name(), &[][..]));
    let derives = graph.derives_from.iter().map(|&(class, parent)| (graph.classes[class].name(), graph.classes[parent].name(), &[][..]));
    vec![
        ("instances.csv", csv_nodes(&graph.instances, "Instance")),
        ("classes.csv", csv_nodes(&graph.classes, "Class")),
        ("feeds.csv", csv_edges([":START_ID(Instance)", ":END_ID(Instance)"], "FEEDS", &FEEDS_PROPERTIES, feeds)),
        ("is_a.csv", csv_edges([":START_ID(Instance)", ":END_ID(Class)"], "IS_A", &[], is_a)),
        ("derives_from.csv", csv_edges([":START_ID(Class)", ":END_ID(Class)"], "DERIVES_FROM", &[], derives)),
    ]
}

// --- GraphML ---

fn xml_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            c => escaped.push(c),
        }
    }
    escaped
}

fn graphml(graph: &Graph, what: &str) -> String {
    let node_columns = columns(graph.classes.iter().chain(&graph.instances));
    let mut text = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    text += "<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"http://graphml.graphdrawing.org/xmlns http://graphml.graphdrawing.org/xmlns/1.0/graphml.xsd\">\n";
    let _ = writeln!(text, "  <desc>{}</desc>", xml_text(&format!("crabsim's export of {what}")));
    text += "  <key id=\"labels\" for=\"node\" attr.name=\"labels\" attr.type=\"string\"/>\n";
    for (key, kind) in &node_columns {
        let _ = writeln!(text, "  <key id=\"n_{key}\" for=\"node\" attr.name=\"{key}\" attr.type=\"{kind}\"/>");
    }
    text += "  <key id=\"type\" for=\"edge\" attr.name=\"type\" attr.type=\"string\"/>\n";
    for (key, kind) in FEEDS_PROPERTIES {
        let _ = writeln!(text, "  <key id=\"e_{key}\" for=\"edge\" attr.name=\"{key}\" attr.type=\"{kind}\"/>");
    }
    text += "  <graph id=\"model\" edgedefault=\"directed\">\n";
    let node = |text: &mut String, id: String, node: &Node| {
        let _ = write!(text, "    <node id=\"{id}\"><data key=\"labels\">:{}</data>", node.labels.join(":"));
        for (key, value) in &node.properties {
            let _ = write!(text, "<data key=\"n_{key}\">{}</data>", xml_text(&value.plain()));
        }
        text.push_str("</node>\n");
    };
    graph.classes.iter().enumerate().for_each(|(i, class)| node(&mut text, format!("c{i}"), class));
    graph.instances.iter().enumerate().for_each(|(i, instance)| node(&mut text, format!("i{i}"), instance));
    let mut edges = 0;
    let mut edge = |text: &mut String, from: String, to: String, kind: &str, properties: &[(&'static str, Value)]| {
        let _ = write!(text, "    <edge id=\"e{edges}\" source=\"{from}\" target=\"{to}\"><data key=\"type\">{kind}</data>");
        for (key, value) in properties {
            let _ = write!(text, "<data key=\"e_{key}\">{}</data>", xml_text(&value.plain()));
        }
        text.push_str("</edge>\n");
        edges += 1;
    };
    for (from, to, properties) in &graph.feeds {
        edge(&mut text, format!("i{from}"), format!("i{to}"), "FEEDS", properties);
    }
    for &(instance, class) in &graph.is_a {
        edge(&mut text, format!("i{instance}"), format!("c{class}"), "IS_A", &[]);
    }
    for &(class, parent) in &graph.derives_from {
        edge(&mut text, format!("c{class}"), format!("c{parent}"), "DERIVES_FROM", &[]);
    }
    text += "  </graph>\n</graphml>\n";
    text
}

// --- export_model ---

pub struct ExportModel;

/// The format to write.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// One `.cypher` script of statements that create the graph in Neo4j or another database that
    /// reads Cypher.
    Cypher,
    /// A folder of five CSV files for `neo4j-admin database import`: instances.csv, classes.csv,
    /// feeds.csv, is_a.csv and derives_from.csv.
    Neo4jCsv,
    /// One `.graphml` file, which graph tools such as Gephi, yEd and NetworkX read.
    Graphml,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportModelInput {
    /// A model file in the models folder, such as `plant/line.json`, exported as
    /// `exports/plant/line.cypher` and so on.
    pub path: Option<String>,
    /// A run, such as `run-3`: the model it ran, exported as `exports/run-3.cypher` and so on.
    pub run: Option<String>,
    pub format: Format,
}

#[derive(Serialize, JsonSchema)]
pub struct WrittenFile {
    /// Relative to the models folder, such as `exports/plant/line.cypher`.
    pub path: String,
    pub bytes: u64,
}

#[derive(Serialize, JsonSchema)]
pub struct ExportModelOutput {
    pub format: Format,
    /// Every file written, replacing any export of the same model in the same format.
    pub files: Vec<WrittenFile>,
    /// How many instance nodes, class nodes (the four built-in classes too) and FEEDS
    /// connections the graph has.
    pub instances: usize,
    pub classes: usize,
    pub connections: usize,
}

impl Operation for ExportModel {
    const NAME: &'static str = "export_model";
    const DESCRIPTION: &'static str = "Writes a valid model, from a file in the models folder (`path`) or the one a run ran (`run`), as a graph for other tools, into the models folder's `exports` folder, overwriting an earlier export of it in the same format. `format` is cypher (one script of Cypher statements, for Neo4j), neo4j-csv (a folder of CSV files for Neo4j's bulk import) or graphml (for Gephi, yEd, NetworkX and others). The graph is the same in every format: Instance nodes, labelled with their built-in class, with name, class, stream, position and every resolved attribute value (times in seconds, such as process_time_s); Class nodes, the built-in ones too, with the values each sets; FEEDS for each connection, from an instance to a successor, with its `exit` (its place in the order the instance offers MUs on, from 1), its `entrance` (its place among the connections into the successor) and its `description` if it has one; IS_A from an instance to its class; and DERIVES_FROM from a class to its parent. Returns the paths and sizes of the files written, not their content, which for a large model is several MB: to answer questions about a model, use describe_model and query_model instead.";
    const EFFECT: Effect = Effect::Destructive;
    type Input = ExportModelInput;
    type Output = ExportModelOutput;

    fn run(catalogue: &Catalogue, input: ExportModelInput) -> Result<ExportModelOutput, OperationError> {
        // Named after the model's path without `.json`, or after the run.
        let stem = match (&input.path, &input.run) {
            (Some(path), None) => {
                let lower = path.to_ascii_lowercase();
                if lower.ends_with(".json") { path[..path.len() - 5].to_string() } else { path.clone() }
            }
            (None, Some(run)) => run.clone(),
            _ => return Err(OperationError::invalid_input("give either `path` or `run`, not both and not neither")),
        };
        let (what, text) = model_text(catalogue, input.path, input.run)?;
        let model = crabsim_model::load(&text).map_err(|problems| {
            let mut error = OperationError::invalid_model(&what, &problems);
            error.message += ", so it can't be exported: fix it first";
            error
        })?;
        let graph = Graph::of(&model);
        let files: Vec<(String, String)> = match input.format {
            Format::Cypher => vec![(format!("{stem}.cypher"), cypher(&graph, &what))],
            Format::Graphml => vec![(format!("{stem}.graphml"), graphml(&graph, &what))],
            Format::Neo4jCsv => neo4j_csv(&graph).into_iter().map(|(name, text)| (format!("{stem}/{name}"), text)).collect(),
        };
        let mut written = Vec::new();
        for (relative, text) in files {
            let file = catalogue.folder().export_file(&relative)?;
            let shown = format!("{EXPORTS_FOLDER}/{relative}");
            let fail = |error: std::io::Error| OperationError::run_error(format!("`{shown}` can't be written: {error}"));
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).map_err(fail)?;
            }
            // Written beside it, then renamed over it, so an export is never seen half-written.
            // Overwriting a .cypher file in place also took 130 ms on Windows, against 1 ms new.
            let mut writing = file.clone().into_os_string();
            writing.push(".writing");
            std::fs::write(&writing, &text).map_err(fail)?;
            std::fs::rename(&writing, &file).map_err(fail)?;
            written.push(WrittenFile { path: shown, bytes: text.len() as u64 });
        }
        Ok(ExportModelOutput {
            format: input.format,
            files: written,
            instances: graph.instances.len(),
            classes: graph.classes.len(),
            connections: graph.feeds.len(),
        })
    }
}
