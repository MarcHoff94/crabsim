//! `describe_model` and `query_model`, by `call(name, JSON)`: small models whose answers are known
//! by hand, a model with problems, a run's model, and on demand the cost on 6,006 objects.

mod common;

use common::Setup;
use serde_json::{json, Value};

/// A Source, a Station that splits the flow three ways, two presses that merge again before one
/// Drain, and a third press into another Drain, with a class derived from a derived class and a
/// class nobody uses.
const BRANCH: &str = r#"{"format_version":2,"description":"Split, press and join","classes":[
  {"name":"Press","parent":"Station","attributes":{"process_time":"0:05","description":"Hydraulic press"}},
  {"name":"Fast press","parent":"Press","attributes":{"process_time":"0:03"}},
  {"name":"Unused","parent":"Station"}],"instances":[
  {"name":"In","class":"Source","attributes":{"interval":"10"}},
  {"name":"Split","class":"Station","attributes":{"process_time":"1"}},
  {"name":"P1","class":"Press"},
  {"name":"P2","class":"Fast press"},
  {"name":"P3","class":"Press","attributes":{"process_time":"0:40"}},
  {"name":"Join","class":"Station","attributes":{"process_time":"2"}},
  {"name":"Out","class":"Drain"},
  {"name":"Scrap","class":"Drain"}],"connections":[
  {"from":"In","to":"Split","exit":1,"entrance":1},
  {"from":"Split","to":"P1","exit":1,"entrance":1},
  {"from":"Split","to":"P2","exit":2,"entrance":1},
  {"from":"Split","to":"P3","exit":3,"entrance":1},
  {"from":"P1","to":"Join","exit":1,"entrance":1},
  {"from":"P2","to":"Join","exit":1,"entrance":2},
  {"from":"P3","to":"Scrap","exit":1,"entrance":1},
  {"from":"Join","to":"Out","exit":1,"entrance":1}]}"#;

/// A rework loop between Work and Check, a Station that nothing feeds, and one that passes
/// nothing on.
const REWORK: &str = r#"{"format_version":2,"instances":[
  {"name":"In","class":"Source","attributes":{"interval":"10"}},
  {"name":"Work","class":"Station","attributes":{"process_time":"5"}},
  {"name":"Check","class":"Station","attributes":{"process_time":"2"}},
  {"name":"Stuck","class":"Station","attributes":{"process_time":"1"}},
  {"name":"Orphan","class":"Station","attributes":{"process_time":"1"}},
  {"name":"Out","class":"Drain"}],"connections":[
  {"from":"In","to":"Work","exit":1,"entrance":1},
  {"from":"Work","to":"Check","exit":1,"entrance":1},
  {"from":"Check","to":"Out","exit":1,"entrance":1},
  {"from":"Check","to":"Work","exit":2,"entrance":2},
  {"from":"Check","to":"Stuck","exit":3,"entrance":1},
  {"from":"Orphan","to":"Out","exit":1,"entrance":2}]}"#;

/// A line: In, then `diamonds` diamonds in a row, each a Station splitting to two that join at
/// the next, then Out. There are 2^diamonds paths from In to Out, all of one length.
fn diamonds(diamonds: usize) -> String {
    let station = |name: String, successors: Vec<String>| json!({ "name": name, "class": "Station", "successors": successors, "attributes": { "process_time": "1" } });
    let mut instances = vec![json!({ "name": "In", "class": "Source", "successors": ["D0"], "attributes": { "interval": "10" } })];
    for i in 0..diamonds {
        instances.push(station(format!("D{i}"), vec![format!("A{i}"), format!("B{i}")]));
        instances.push(station(format!("A{i}"), vec![format!("D{}", i + 1)]));
        instances.push(station(format!("B{i}"), vec![format!("D{}", i + 1)]));
    }
    instances.push(station(format!("D{diamonds}"), vec!["Out".to_string()]));
    instances.push(json!({ "name": "Out", "class": "Drain" }));
    common::connected(json!({ "format_version": 1, "instances": instances })).to_string()
}

fn setup(name: &str) -> Setup {
    let setup = Setup::new(name);
    setup.folder.write("branch.json", BRANCH);
    setup.folder.write("rework.json", REWORK);
    setup.folder.write("broken.json", common::BROKEN);
    setup
}

fn describe(setup: &Setup, path: &str) -> Value {
    setup.ok("describe_model", json!({ "path": path }))
}

fn query(setup: &Setup, path: &str, query: Value) -> Value {
    setup.ok("query_model", json!({ "path": path, "query": query }))
}

/// The `name` of each entry of a list.
fn names(list: &Value) -> Vec<&str> {
    list.as_array().unwrap_or_else(|| panic!("not a list: {list}")).iter().map(|entry| entry["name"].as_str().unwrap()).collect()
}

/// `name` and `distance` of each entry of a list.
fn distances(list: &Value) -> Vec<(&str, u64)> {
    list.as_array().unwrap().iter().map(|entry| (entry["name"].as_str().unwrap(), entry["distance"].as_u64().unwrap())).collect()
}

fn strings(list: &Value) -> Vec<&str> {
    list.as_array().unwrap_or_else(|| panic!("not a list: {list}")).iter().map(|name| name.as_str().unwrap()).collect()
}

// ---------- describe_model ----------

#[test]
fn a_summary_gives_the_shape_of_a_branching_model() {
    let setup = setup("describe-branch");
    let summary = describe(&setup, "branch.json");
    assert_eq!(summary["valid"], true);
    assert_eq!(summary["description"], "Split, press and join");
    assert_eq!(summary["instances"], 8);
    let classes: Vec<(&str, &str, u64)> = summary["classes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|class| (class["class"].as_str().unwrap(), class["built_in_class"].as_str().unwrap(), class["instances"].as_u64().unwrap()))
        .collect();
    assert_eq!(
        classes,
        [
            ("Source", "Source", 1),
            ("Station", "Station", 2),
            ("Drain", "Drain", 2),
            ("Press", "Station", 2),
            ("Fast press", "Station", 1),
            ("Unused", "Station", 0)
        ],
        "built-in classes with instances first, then the model's own in file order, counting only instances of the class itself"
    );
    assert_eq!(summary["sources"], json!({ "count": 1, "names": ["In"] }));
    assert_eq!(summary["drains"], json!({ "count": 2, "names": ["Out", "Scrap"] }));
    assert_eq!(summary["branch_points"], json!({ "count": 1, "widest": [{ "name": "Split", "successors": 3 }] }));
    assert_eq!(summary["merge_points"], json!({ "count": 1, "widest": [{ "name": "Join", "predecessors": 2 }] }));
    assert_eq!(
        summary["flows"],
        json!([{ "source": "In", "drains": [
            { "drain": "Out", "fewest_objects": 3, "most_objects": 3, "through_loop": false },
            { "drain": "Scrap", "fewest_objects": 2, "most_objects": 2, "through_loop": false }
        ] }])
    );
    assert_eq!(summary["loops"], json!({ "count": 0, "examples": [] }));
    assert_eq!(summary["unreachable"], json!({ "count": 0, "names": [] }));
    assert_eq!(summary["dead_ends"], json!({ "count": 0, "names": [] }));
    assert_eq!(summary["problems"], json!({ "count": 0, "first": [] }));
}

#[test]
fn a_summary_finds_loops_unreachable_instances_and_dead_ends() {
    let setup = setup("describe-rework");
    let summary = describe(&setup, "rework.json");
    assert_eq!(summary["valid"], true);
    assert_eq!(summary["loops"]["count"], 1);
    let example = &summary["loops"]["examples"][0];
    assert_eq!(example["objects"], 2);
    assert_eq!(strings(&example["cycle"]), ["Work", "Check"], "a cycle starts at its first instance in file order");
    assert_eq!(summary["unreachable"], json!({ "count": 1, "names": ["Orphan"] }));
    assert_eq!(summary["dead_ends"], json!({ "count": 1, "names": ["Stuck"] }));
    assert_eq!(summary["branch_points"], json!({ "count": 1, "widest": [{ "name": "Check", "successors": 3 }] }));
    assert_eq!(
        summary["merge_points"],
        json!({ "count": 2, "widest": [{ "name": "Work", "predecessors": 2 }, { "name": "Out", "predecessors": 2 }] }),
        "equally wide ones in file order"
    );
    assert_eq!(
        summary["flows"],
        json!([{ "source": "In", "drains": [{ "drain": "Out", "fewest_objects": 2, "through_loop": true }] }]),
        "through a loop, an MU can pass any number of objects, so there is no most"
    );
}

#[test]
fn the_widest_branch_and_merge_points_come_first() {
    let setup = setup("describe-widest");
    let model = common::connected(json!({ "format_version": 1, "instances": [
        { "name": "In", "class": "Source", "successors": ["A", "Hub"], "attributes": { "interval": "10" } },
        { "name": "A", "class": "Station", "successors": ["Out 1"] },
        { "name": "Hub", "class": "Station", "successors": ["X", "Y", "Z", "W"] },
        { "name": "X", "class": "Station", "successors": ["Out 1"] },
        { "name": "Y", "class": "Station", "successors": ["Out 2"] },
        { "name": "Z", "class": "Station", "successors": ["Out 2"] },
        { "name": "W", "class": "Station", "successors": ["Out 2"] },
        { "name": "Out 1", "class": "Drain" },
        { "name": "Out 2", "class": "Drain" }
    ]}));
    setup.folder.write("widest.json", &model.to_string());
    let summary = describe(&setup, "widest.json");
    assert_eq!(summary["branch_points"]["widest"], json!([{ "name": "Hub", "successors": 4 }, { "name": "In", "successors": 2 }]));
    assert_eq!(summary["merge_points"]["widest"], json!([{ "name": "Out 2", "predecessors": 3 }, { "name": "Out 1", "predecessors": 2 }]));
}

#[test]
fn a_summary_of_a_model_with_problems_says_what_it_can_and_lists_them() {
    let setup = setup("describe-broken");
    let summary = describe(&setup, "broken.json");
    assert_eq!(summary["valid"], false);
    assert_eq!(summary["instances"], 2);
    assert_eq!(summary["sources"]["names"], json!(["In"]));
    assert_eq!(summary["drains"]["names"], json!(["Out"]));
    assert_eq!(summary["dead_ends"]["names"], json!(["In"]), "its only successor doesn't exist, so it's skipped");
    assert_eq!(summary["unreachable"]["names"], json!(["Out"]));
    let expected = setup.ok("validate_model", json!({ "path": "broken.json" }))["problems"].clone();
    assert_eq!(summary["problems"]["count"], expected.as_array().unwrap().len());
    assert_eq!(summary["problems"]["first"], expected);

    setup.folder.write("not-json.json", "{ this isn't");
    let summary = describe(&setup, "not-json.json");
    assert_eq!((summary["valid"].clone(), summary["instances"].clone()), (json!(false), json!(0)));
    assert!(summary["problems"]["first"][0]["message"].as_str().unwrap().contains("isn't JSON"), "{summary}");
}

#[test]
fn a_summary_stays_the_same_size_whatever_the_model() {
    let setup = setup("describe-size");
    // 40 Sources, each into its own Drain and into a shared one, and 60 Stations nothing feeds
    // that pass nothing on.
    let mut instances = Vec::new();
    for i in 0..40 {
        instances.push(json!({ "name": format!("In {i}"), "class": "Source", "successors": [format!("Out {i}"), "Shared"], "attributes": { "interval": "10" } }));
        instances.push(json!({ "name": format!("Out {i}"), "class": "Drain" }));
    }
    instances.push(json!({ "name": "Shared", "class": "Drain" }));
    for i in 0..60 {
        instances.push(json!({ "name": format!("Idle {i}"), "class": "Station" }));
    }
    setup.folder.write("many.json", &common::connected(json!({ "format_version": 1, "instances": instances })).to_string());
    let summary = describe(&setup, "many.json");
    assert_eq!(summary["sources"]["count"], 40);
    assert_eq!(summary["sources"]["names"].as_array().unwrap().len(), 10);
    assert_eq!(summary["drains"]["count"], 41);
    assert_eq!(summary["drains"]["names"].as_array().unwrap().len(), 10);
    assert_eq!(summary["branch_points"]["count"], 40);
    assert_eq!(summary["branch_points"]["widest"].as_array().unwrap().len(), 10);
    assert_eq!(summary["merge_points"]["widest"], json!([{ "name": "Shared", "predecessors": 40 }]));
    assert_eq!(summary["flows"].as_array().unwrap().len(), 10, "the flows of the Sources it lists");
    assert_eq!(summary["unreachable"]["count"], 60);
    assert_eq!(summary["unreachable"]["names"].as_array().unwrap().len(), 10);
    assert_eq!(summary["dead_ends"]["count"], 60);
}

// ---------- query_model: neighbours, upstream and downstream ----------

#[test]
fn neighbours_are_the_predecessors_and_successors_with_their_classes() {
    let setup = setup("neighbours");
    let answer = query(&setup, "branch.json", json!({ "kind": "neighbours", "name": "P2" }));
    assert_eq!(answer["kind"], "neighbours");
    assert_eq!(answer["predecessors"], json!([{ "name": "Split", "class": "Station", "built_in_class": "Station" }]));
    assert_eq!(answer["successors"], json!([{ "name": "Join", "class": "Station", "built_in_class": "Station" }]));
    let join = query(&setup, "branch.json", json!({ "kind": "neighbours", "name": "Join" }));
    assert_eq!(names(&join["predecessors"]), ["P1", "P2"]);
}

#[test]
fn upstream_and_downstream_give_every_instance_within_the_steps_with_its_distance() {
    let setup = setup("upstream");
    let up = query(&setup, "branch.json", json!({ "kind": "upstream", "name": "Out" }));
    assert_eq!(distances(&up["instances"]), [("Join", 1), ("P1", 2), ("P2", 2), ("Split", 3), ("In", 4)]);
    assert_eq!((up["count"].clone(), up["truncated"].clone()), (json!(5), json!(false)));
    assert_eq!(up["instances"][1]["class"], "Press");
    assert_eq!(up["instances"][1]["built_in_class"], "Station");

    let near = query(&setup, "branch.json", json!({ "kind": "upstream", "name": "Out", "steps": 2 }));
    assert_eq!(distances(&near["instances"]), [("Join", 1), ("P1", 2), ("P2", 2)]);

    let down = query(&setup, "branch.json", json!({ "kind": "downstream", "name": "Split" }));
    assert_eq!(distances(&down["instances"]), [("P1", 1), ("P2", 1), ("P3", 1), ("Join", 2), ("Scrap", 2), ("Out", 3)]);
}

#[test]
fn in_a_loop_an_instance_is_never_its_own_upstream_or_downstream() {
    let setup = setup("upstream-loop");
    let up = query(&setup, "rework.json", json!({ "kind": "upstream", "name": "Work" }));
    assert_eq!(distances(&up["instances"]), [("In", 1), ("Check", 1)]);
    let down = query(&setup, "rework.json", json!({ "kind": "downstream", "name": "Check" }));
    assert_eq!(distances(&down["instances"]), [("Work", 1), ("Stuck", 1), ("Out", 1)]);
}

// ---------- query_model: paths ----------

#[test]
fn paths_give_the_shortest_then_the_others_and_how_many_there_are() {
    let setup = setup("paths");
    let answer = query(&setup, "branch.json", json!({ "kind": "paths", "from": "In", "to": "Out" }));
    assert_eq!(answer["shortest"], json!(["In", "Split", "P1", "Join", "Out"]));
    assert_eq!(answer["others"], json!([["In", "Split", "P2", "Join", "Out"]]));
    assert_eq!((answer["count"].clone(), answer["more"].clone()), (json!(2), json!(false)));

    let none = query(&setup, "branch.json", json!({ "kind": "paths", "from": "P3", "to": "Out" }));
    assert_eq!(none["shortest"], Value::Null);
    assert_eq!((none["others"].clone(), none["count"].clone()), (json!([]), json!(0)));
}

#[test]
fn a_path_never_passes_an_object_twice() {
    let setup = setup("paths-loop");
    let answer = query(&setup, "rework.json", json!({ "kind": "paths", "from": "In", "to": "Out" }));
    assert_eq!(answer["shortest"], json!(["In", "Work", "Check", "Out"]));
    assert_eq!((answer["others"].clone(), answer["count"].clone()), (json!([]), json!(1)));

    // Two ways through a diamond whose end leads back to its start.
    let model = common::connected(json!({ "format_version": 1, "instances": [
        { "name": "In", "class": "Source", "successors": ["A"], "attributes": { "interval": "10" } },
        { "name": "A", "class": "Station", "successors": ["B", "C"], "attributes": { "process_time": "1" } },
        { "name": "B", "class": "Station", "successors": ["D"], "attributes": { "process_time": "1" } },
        { "name": "C", "class": "Station", "successors": ["D"], "attributes": { "process_time": "1" } },
        { "name": "D", "class": "Station", "successors": ["Out", "A"], "attributes": { "process_time": "1" } },
        { "name": "Out", "class": "Drain" }
    ]}));
    setup.folder.write("diamond-loop.json", &model.to_string());
    let answer = query(&setup, "diamond-loop.json", json!({ "kind": "paths", "from": "In", "to": "Out" }));
    assert_eq!(answer["shortest"], json!(["In", "A", "B", "D", "Out"]));
    assert_eq!(answer["others"], json!([["In", "A", "C", "D", "Out"]]));
    assert_eq!((answer["count"].clone(), answer["more"].clone()), (json!(2), json!(false)));
}

#[test]
fn paths_are_counted_up_to_ten_thousand() {
    let setup = setup("paths-many");
    setup.folder.write("small-diamonds.json", &diamonds(3));
    let few = query(&setup, "small-diamonds.json", json!({ "kind": "paths", "from": "In", "to": "Out" }));
    assert_eq!((few["count"].clone(), few["more"].clone()), (json!(8), json!(false)));
    assert_eq!(few["others"].as_array().unwrap().len(), 7);

    setup.folder.write("diamonds.json", &diamonds(14));
    let many = query(&setup, "diamonds.json", json!({ "kind": "paths", "from": "In", "to": "Out" }));
    assert_eq!((many["count"].clone(), many["more"].clone()), (json!(10_000), json!(true)), "2^14 = 16,384 paths");
    let others = many["others"].as_array().unwrap();
    assert_eq!(others.len(), 20);
    let mut seen: Vec<&Value> = others.iter().collect();
    seen.push(&many["shortest"]);
    seen.sort_by_key(|path| path.to_string());
    seen.dedup();
    assert_eq!(seen.len(), 21, "every path listed is a different one");
}

#[test]
fn the_other_paths_come_shortest_first() {
    let setup = setup("paths-order");
    // A long way round first in the successor order, then a short cut.
    let model = common::connected(json!({ "format_version": 1, "instances": [
        { "name": "In", "class": "Source", "successors": ["L1", "Mid", "Out"], "attributes": { "interval": "10" } },
        { "name": "L1", "class": "Station", "successors": ["L2"] },
        { "name": "L2", "class": "Station", "successors": ["Out"] },
        { "name": "Mid", "class": "Station", "successors": ["Out"] },
        { "name": "Out", "class": "Drain" }
    ]}));
    setup.folder.write("order.json", &model.to_string());
    let answer = query(&setup, "order.json", json!({ "kind": "paths", "from": "In", "to": "Out" }));
    assert_eq!(answer["shortest"], json!(["In", "Out"]));
    assert_eq!(answer["others"], json!([["In", "Mid", "Out"], ["In", "L1", "L2", "Out"]]));
}

/// Three routes from In to Out with known totals of process time: In, B, D, Out takes 33 s,
/// In, A, C, Out 15 s and In, B, C, Out 8 s. In and Out have no process time.
const PLAN: &str = r#"{"format_version":2,"instances":[
  {"name":"In","class":"Source","attributes":{"interval":"10"}},
  {"name":"A","class":"Station","attributes":{"process_time":"10"}},
  {"name":"B","class":"Station","attributes":{"process_time":"3"}},
  {"name":"C","class":"Station","attributes":{"process_time":"5"}},
  {"name":"D","class":"Station","attributes":{"process_time":"30"}},
  {"name":"Out","class":"Drain"}],"connections":[
  {"from":"In","to":"A","exit":1,"entrance":1},
  {"from":"In","to":"B","exit":2,"entrance":1},
  {"from":"A","to":"C","exit":1,"entrance":1},
  {"from":"B","to":"C","exit":1,"entrance":2},
  {"from":"B","to":"D","exit":2,"entrance":1},
  {"from":"C","to":"Out","exit":1,"entrance":1},
  {"from":"D","to":"Out","exit":1,"entrance":2}]}"#;

/// The routes and totals of a weighted answer.
fn weighted(answer: &Value) -> Vec<(Vec<&str>, f64)> {
    answer["weighted"]["routes"].as_array().unwrap_or_else(|| panic!("no weighted routes: {answer:#}")).iter().map(|route| (strings(&route["instances"]), route["total"].as_f64().unwrap())).collect()
}

#[test]
fn distribution_gives_how_an_instance_ranks_its_successors() {
    let setup = setup("distribution");
    setup.ok("write_model", json!({ "path": "plan.json", "text": PLAN }));
    // Round robin by default, in exit order.
    let answer = query(&setup, "plan.json", json!({ "kind": "distribution", "name": "In" }));
    common::assert_fits_its_schema(&setup.catalogue, "query_model", &answer);
    assert_eq!(answer["strategy"], "round_robin");
    assert_eq!(names(&answer["successors"]), ["A", "B"]);

    // Weighted by the heaviest route onward: B (3 s) then D (30 s) against A (10 s) then C (5 s).
    setup.ok("edit_model", json!({ "path": "plan.json", "changes": [{
        "change": "set_attribute", "target": { "instance": "In" }, "attribute": "distribution",
        "value": { "weighted": { "by": "process_time", "order": "largest" } } }] }));
    let answer = query(&setup, "plan.json", json!({ "kind": "distribution", "name": "In" }));
    assert_eq!(answer["strategy"], "weighted");
    assert_eq!(answer["weighted"], json!({ "weighted": { "by": "process_time", "order": "largest", "rank": "route" } }));
    assert_eq!(
        answer["successors"],
        json!([{ "name": "B", "exit": 2, "place": 1, "total": 33.0 }, { "name": "A", "exit": 1, "place": 2, "total": 15.0 }])
    );

    // An unknown weight is refused by the model file's own check.
    let refused = setup.err("edit_model", json!({ "path": "plan.json", "changes": [{
        "change": "set_attribute", "target": { "instance": "In" }, "attribute": "distribution",
        "value": { "weighted": { "by": "speed" } } }] }));
    assert_eq!(refused["kind"], "invalid_model");
}

#[test]
fn weighted_paths_come_by_their_total_the_critical_path_first_when_largest() {
    let setup = setup("paths-weighted");
    setup.folder.write("plan.json", PLAN);
    let largest = query(&setup, "plan.json", json!({ "kind": "paths", "from": "In", "to": "Out", "weight": { "attribute": "process_time" }, "order": "largest" }));
    common::assert_fits_its_schema(&setup.catalogue, "query_model", &largest);
    assert_eq!(
        weighted(&largest),
        [(vec!["In", "B", "D", "Out"], 33.0), (vec!["In", "A", "C", "Out"], 15.0), (vec!["In", "B", "C", "Out"], 8.0)]
    );
    assert_eq!(largest["weighted"]["by"], "process_time, in seconds");
    assert_eq!(largest["weighted"]["order"], "largest");
    assert_eq!(strings(&largest["weighted"]["weightless"]), ["In", "Out"], "a Source and a Drain have no process time");
    assert_eq!(largest["weighted"]["exact"], true);
    assert_eq!(largest["count"], 3, "the unweighted answer is still there");

    let smallest = query(&setup, "plan.json", json!({ "kind": "paths", "from": "In", "to": "Out", "weight": { "attribute": "process_time" } }));
    assert_eq!(
        weighted(&smallest),
        [(vec!["In", "B", "C", "Out"], 8.0), (vec!["In", "A", "C", "Out"], 15.0), (vec!["In", "B", "D", "Out"], 33.0)],
        "smallest first by default"
    );

    // An inherited value weighs as much as one set on the instance.
    let by_interval = query(&setup, "plan.json", json!({ "kind": "paths", "from": "In", "to": "C", "weight": { "attribute": "interval" } }));
    assert_eq!(weighted(&by_interval)[0].1, 10.0);
    assert_eq!(strings(&by_interval["weighted"]["weightless"]), ["A", "B", "C"]);

    for (query, says) in [
        (json!({ "kind": "paths", "from": "In", "to": "Out", "order": "largest" }), "give a `weight` too"),
        (json!({ "kind": "paths", "from": "In", "to": "Out", "weight": { "attribute": "description" } }), "not `description`"),
        (json!({ "kind": "paths", "from": "In", "to": "Out", "weight": { "figure": "working_share" } }), "give `run`"),
    ] {
        let refused = setup.err("query_model", json!({ "path": "plan.json", "query": query }));
        assert_eq!(refused["kind"], "invalid_input");
        assert!(refused["message"].as_str().unwrap().contains(says), "{refused}");
    }
}

#[test]
fn paths_are_weighed_by_values_given_by_name() {
    let setup = setup("paths-values");
    setup.folder.write("plan.json", PLAN);
    let answer = query(&setup, "plan.json", json!({ "kind": "paths", "from": "In", "to": "Out", "weight": { "values": { "A": 0.9, "C": 0.2, "B": 0.1, "D": 0.4 } }, "order": "largest" }));
    let routes = weighted(&answer);
    assert_eq!(routes[0], (vec!["In", "A", "C", "Out"], 1.1));
    assert_eq!(answer["weighted"]["by"], "the values given");
    assert_eq!(strings(&answer["weighted"]["weightless"]), ["In", "Out"], "left out, so 0");
    let refused = setup.err("query_model", json!({ "path": "plan.json", "query": { "kind": "paths", "from": "In", "to": "Out", "weight": { "values": { "A": -1.0 } } } }));
    assert!(refused["message"].as_str().unwrap().contains("0 or more"), "{refused}");
}

#[test]
fn the_heaviest_routes_through_a_loop_pass_no_object_twice() {
    let setup = setup("paths-weighted-loop");
    // Two ways through a diamond whose end leads back to its start, the way through C heavier.
    let model = common::connected(json!({ "format_version": 1, "instances": [
        { "name": "In", "class": "Source", "successors": ["A"], "attributes": { "interval": "10" } },
        { "name": "A", "class": "Station", "successors": ["B", "C"], "attributes": { "process_time": "1" } },
        { "name": "B", "class": "Station", "successors": ["D"], "attributes": { "process_time": "2" } },
        { "name": "C", "class": "Station", "successors": ["D"], "attributes": { "process_time": "7" } },
        { "name": "D", "class": "Station", "successors": ["Out", "A"], "attributes": { "process_time": "1" } },
        { "name": "Out", "class": "Drain" }
    ]}));
    setup.folder.write("diamond-loop.json", &model.to_string());
    let answer = query(&setup, "diamond-loop.json", json!({ "kind": "paths", "from": "In", "to": "Out", "weight": { "attribute": "process_time" }, "order": "largest" }));
    assert_eq!(weighted(&answer), [(vec!["In", "A", "C", "D", "Out"], 9.0), (vec!["In", "A", "B", "D", "Out"], 4.0)]);
    assert_eq!(answer["weighted"]["exact"], true, "two routes are quickly walked");

    // Many diamonds still give 21 routes, the heaviest first, all different.
    setup.folder.write("diamonds.json", &diamonds(14));
    let many = query(&setup, "diamonds.json", json!({ "kind": "paths", "from": "In", "to": "Out", "weight": { "attribute": "process_time" }, "order": "largest" }));
    let routes = weighted(&many);
    assert_eq!(routes.len(), 21);
    assert!(routes.iter().all(|(_, total)| *total == 29.0), "every route through 14 diamonds passes 29 Stations of 1 s");
}

#[test]
fn paths_are_weighed_by_a_runs_figures_the_mean_over_its_observations() {
    let setup = setup("paths-figure");
    setup.folder.write("plan.json", PLAN);
    let run = setup.start(json!({ "model": "plan.json", "seed": 1, "length": "4:00:00", "observations": 2 }));
    setup.finished(&run);
    let mean = |object: &str, statistic: &str| {
        let stored = setup.stored(&run, object);
        stored.values().map(|figures| figures[statistic]).sum::<f64>() / stored.len() as f64
    };
    let answer = setup.ok("query_model", json!({ "run": run, "query": { "kind": "paths", "from": "In", "to": "Out", "weight": { "figure": "working_share" }, "order": "largest" } }));
    let routes = weighted(&answer);
    assert_eq!(routes.len(), 3);
    for (route, total) in &routes {
        let expected: f64 = route.iter().map(|object| mean(object, "working_share")).sum();
        assert!((total - expected).abs() <= expected * 1e-3, "{route:?} totals {total}, its means add up to {expected}");
    }
    assert!(routes.windows(2).all(|pair| pair[0].1 >= pair[1].1), "largest first: {routes:?}");
    assert_eq!(answer["weighted"]["by"], "working_share, the mean over the run's observations");

    // Only a Drain has a lead time, so every other object weighs 0 and is named.
    let lead = setup.ok("query_model", json!({ "run": run, "query": { "kind": "paths", "from": "In", "to": "Out", "weight": { "figure": "mean_lead_time_s" } } }));
    assert_eq!(strings(&lead["weighted"]["weightless"]), ["In", "A", "B", "C", "D"]);
    let out = mean("Out", "mean_lead_time_s");
    assert!(weighted(&lead).iter().all(|(_, total)| (total - out).abs() <= out * 1e-3), "{lead:#}");

    let unknown = setup.err("query_model", json!({ "run": run, "query": { "kind": "paths", "from": "In", "to": "Out", "weight": { "figure": "speed" } } }));
    assert!(unknown["message"].as_str().unwrap().contains("working_share"), "the figures there are: {unknown}");
}

// ---------- query_model: subgraph_around ----------

#[test]
fn the_neighbourhood_follows_connections_either_way_and_gives_the_connections_between_its_instances() {
    let setup = setup("subgraph");
    let one = query(&setup, "branch.json", json!({ "kind": "subgraph_around", "name": "P1", "steps": 1 }));
    assert_eq!(distances(&one["nodes"]), [("P1", 0), ("Split", 1), ("Join", 1)]);
    assert_eq!(
        one["edges"],
        json!([{ "from": "Split", "to": "P1", "exit": 1, "entrance": 1 }, { "from": "P1", "to": "Join", "exit": 1, "entrance": 1 }]),
        "each connection as the file has it"
    );

    let two = query(&setup, "branch.json", json!({ "kind": "subgraph_around", "name": "P1" }));
    assert_eq!(two["steps"], 2, "2 by default");
    assert_eq!(
        distances(&two["nodes"]),
        [("P1", 0), ("Split", 1), ("Join", 1), ("In", 2), ("P2", 2), ("P3", 2), ("Out", 2)],
        "by distance, then in file order"
    );
    assert_eq!(
        two["edges"],
        json!([
            { "from": "In", "to": "Split", "exit": 1, "entrance": 1 },
            { "from": "Split", "to": "P1", "exit": 1, "entrance": 1 },
            { "from": "Split", "to": "P2", "exit": 2, "entrance": 1 },
            { "from": "Split", "to": "P3", "exit": 3, "entrance": 1 },
            { "from": "P1", "to": "Join", "exit": 1, "entrance": 1 },
            { "from": "P2", "to": "Join", "exit": 1, "entrance": 2 },
            { "from": "Join", "to": "Out", "exit": 1, "entrance": 1 }
        ]),
        "edges in file order of their `from`, then by exit, leaving out P3 to Scrap"
    );
    assert_eq!(two["truncated"], false);
}

// ---------- query_model: find_instances ----------

fn found(setup: &Setup, criteria: Value) -> Vec<String> {
    let mut query = json!({ "kind": "find_instances" });
    query.as_object_mut().unwrap().extend(criteria.as_object().unwrap().clone());
    let answer = setup.ok("query_model", json!({ "path": "branch.json", "query": query }));
    names(&answer["instances"]).into_iter().map(str::to_string).collect()
}

#[test]
fn instances_are_found_by_name_class_description_and_attribute_values() {
    let setup = setup("find");
    assert_eq!(found(&setup, json!({ "class": "Press" })), ["P1", "P2", "P3"], "a class includes those derived from it");
    assert_eq!(found(&setup, json!({ "class": "Station" })), ["Split", "P1", "P2", "P3", "Join"]);
    assert_eq!(found(&setup, json!({ "name_contains": "p" })), ["Split", "P1", "P2", "P3", "Scrap"], "ignoring case");
    assert_eq!(found(&setup, json!({ "name_contains": "p", "class": "Drain" })), ["Scrap"], "criteria combine with AND");
    assert_eq!(found(&setup, json!({ "description_contains": "HYDRAULIC" })), ["P1", "P2", "P3"], "descriptions are inherited");
    assert_eq!(found(&setup, json!({ "conditions": [{ "attribute": "process_time", "op": ">", "value": "0:04" }] })), ["P1", "P3"]);
    assert_eq!(found(&setup, json!({ "conditions": [{ "attribute": "process_time", "op": "<=", "value": "2" }] })), ["Split", "Join"]);
    assert_eq!(found(&setup, json!({ "conditions": [{ "attribute": "process_time", "op": "=", "value": "00:00:05" }] })), ["P1"]);
    assert_eq!(found(&setup, json!({ "conditions": [{ "attribute": "interval", "op": ">=", "value": "0" }] })), ["In"], "only instances that have the attribute");
    assert_eq!(found(&setup, json!({ "set_on_instance": "process_time" })), ["Split", "P3", "Join"]);
}

#[test]
fn found_instances_come_with_their_matched_values_as_a_file_writes_them() {
    let setup = setup("find-values");
    let answer = query(&setup, "branch.json", json!({ "kind": "find_instances", "conditions": [{ "attribute": "process_time", "op": ">", "value": "0:30" }] }));
    assert_eq!(answer["instances"], json!([{ "name": "P3", "class": "Press", "built_in_class": "Station", "values": { "process_time": "00:00:40" } }]));
    assert_eq!((answer["count"].clone(), answer["truncated"].clone()), (json!(1), json!(false)));
}

#[test]
fn a_condition_that_cant_be_compared_is_refused() {
    let setup = setup("find-refused");
    let unknown = setup.err("query_model", json!({ "path": "branch.json", "query": { "kind": "find_instances", "conditions": [{ "attribute": "speed", "op": ">", "value": "1" }] } }));
    assert_eq!(unknown["kind"], "invalid_input");
    assert!(unknown["message"].as_str().unwrap().contains("speed"), "{unknown}");
    let not_a_time = setup.err("query_model", json!({ "path": "branch.json", "query": { "kind": "find_instances", "conditions": [{ "attribute": "process_time", "op": ">", "value": "soon" }] } }));
    assert_eq!(not_a_time["kind"], "invalid_input");
    let unordered = setup.err("query_model", json!({ "path": "branch.json", "query": { "kind": "find_instances", "conditions": [{ "attribute": "description", "op": "<", "value": "x" }] } }));
    assert_eq!(unordered["kind"], "invalid_input");
}

// ---------- query_model: class_tree ----------

#[test]
fn the_class_tree_runs_from_the_built_in_classes_down() {
    let setup = setup("class-tree");
    let tree = query(&setup, "branch.json", json!({ "kind": "class_tree" }));
    let classes = tree["classes"].as_array().unwrap();
    let outline: Vec<(&str, u64)> = classes.iter().map(|class| (class["name"].as_str().unwrap(), class["depth"].as_u64().unwrap())).collect();
    assert_eq!(outline, [("Source", 0), ("Station", 0), ("Press", 1), ("Fast press", 2), ("Unused", 1), ("Buffer", 0), ("Drain", 0)]);
    let press = classes.iter().find(|class| class["name"] == "Press").unwrap();
    assert_eq!(press["parent"], "Station");
    assert_eq!(press["sets"], json!({ "process_time": "00:00:05", "description": "Hydraulic press" }));
    assert_eq!(press["derived"], json!(["Fast press"]));
    assert_eq!((press["instances_direct"].clone(), press["instances_total"].clone()), (json!(2), json!(3)));
    assert!(press.get("instances").is_none(), "names only when asked for");
    let station = classes.iter().find(|class| class["name"] == "Station").unwrap();
    assert_eq!((station["instances_direct"].clone(), station["instances_total"].clone()), (json!(2), json!(5)));
    assert_eq!(station["derived"], json!(["Press", "Unused"]));
}

#[test]
fn the_class_tree_below_one_class_can_name_the_instances() {
    let setup = setup("class-tree-below");
    let tree = query(&setup, "branch.json", json!({ "kind": "class_tree", "class": "Press", "instances": true }));
    let classes = tree["classes"].as_array().unwrap();
    assert_eq!(names(&tree["classes"]), ["Press", "Fast press"]);
    assert_eq!(classes[0]["depth"], 0, "depth counts from the class asked for");
    assert_eq!(classes[0]["instances"], json!(["P1", "P3"]));
    assert_eq!(classes[1]["instances"], json!(["P2"]));
}

// ---------- query_model: refusals, and a run's model ----------

#[test]
fn a_query_on_a_model_with_problems_or_an_unknown_name_is_refused() {
    let setup = setup("query-refused");
    let broken = setup.err("query_model", json!({ "path": "broken.json", "query": { "kind": "neighbours", "name": "In" } }));
    assert_eq!(broken["kind"], "invalid_model");
    assert_eq!(broken["problems"].as_array().unwrap().len(), 3);
    let unknown = setup.err("query_model", json!({ "path": "branch.json", "query": { "kind": "upstream", "name": "P9" } }));
    assert_eq!(unknown["kind"], "not_found");
    assert!(unknown["message"].as_str().unwrap().contains("P9"), "{unknown}");
    let class = setup.err("query_model", json!({ "path": "branch.json", "query": { "kind": "class_tree", "class": "Lathe" } }));
    assert_eq!(class["kind"], "not_found");
    let both = setup.err("describe_model", json!({ "path": "branch.json", "run": "run-1" }));
    assert_eq!(both["kind"], "invalid_input");
    let neither = setup.err("query_model", json!({ "query": { "kind": "neighbours", "name": "In" } }));
    assert_eq!(neither["kind"], "invalid_input");
}

#[test]
fn a_runs_model_is_the_one_it_ran_even_after_the_file_changed() {
    let setup = setup("run-model");
    setup.ok("adopt_model", json!({ "path": "branch.json" }));
    let run = setup.ok("start_run", json!({ "model": "branch.json", "length": "1:00", "observations": 1, "wait": 30 }))["run"].as_str().unwrap().to_string();
    setup.ok("edit_model", json!({ "path": "branch.json", "changes": [{ "change": "remove_instance", "name": "P3" }] }));

    assert_eq!(describe(&setup, "branch.json")["instances"], 7);
    let summary = setup.ok("describe_model", json!({ "run": run }));
    assert_eq!(summary["instances"], 8);
    let down = setup.ok("query_model", json!({ "run": run, "query": { "kind": "downstream", "name": "P3" } }));
    assert_eq!(distances(&down["instances"]), [("Scrap", 1)]);
    let missing = setup.err("describe_model", json!({ "run": "run-99" }));
    assert_eq!(missing["kind"], "not_found");
}

// ---------- Cost ----------

/// What each costs on the benchmark's model of 429 copies of the demo (6,006 objects). Timing
/// only, so on demand in release:
/// `cargo test --release -p crabsim-catalogue --test graph -- --ignored --nocapture`.
#[test]
#[ignore]
fn describe_and_query_on_6006_objects() {
    let demo = crabsim_model::load(&common::demo_text()).unwrap();
    let mut model = crabsim_model::Model { description: Some("429 copies of the demo model".into()), classes: demo.classes.clone(), ..Default::default() };
    for copy in 0..429 {
        let rename = |name: &str| format!("Copy {copy} / {name}");
        for instance in &demo.instances {
            let mut instance = instance.clone();
            instance.name = rename(&instance.name);
            instance.stream = instance.stream.map(|stream| stream + 100 * copy as u64);
            model.instances.push(instance);
        }
        for connection in &demo.connections {
            let mut connection = connection.clone();
            connection.from = rename(&connection.from);
            connection.to = rename(&connection.to);
            model.connections.push(connection);
        }
    }
    let setup = Setup::new("graph-large");
    setup.folder.write("large.json", &model.save());
    let first = demo.instances.first().unwrap().name.clone();
    let last = demo.instances.last().unwrap().name.clone();
    let middle = demo.instances[demo.instances.len() / 2].name.clone();
    let at = |copy: usize, name: &str| format!("Copy {copy} / {name}");

    // Independent counts: everything in the last copy but its last object is upstream of it, and
    // nothing of any other copy.
    let up = setup.ok("query_model", json!({ "path": "large.json", "query": { "kind": "upstream", "name": at(428, &last) } }));
    assert_eq!(up["count"], demo.instances.len() - 1);
    assert!(names(&up["instances"]).iter().all(|name| name.starts_with("Copy 428 / ")));
    let summary = setup.ok("describe_model", json!({ "path": "large.json" }));
    assert_eq!(summary["instances"], 6006);
    println!("describe_model's summary: {} bytes", summary.to_string().len());

    let time = |what: &str, name: &str, input: Value| {
        let mut times = Vec::new();
        for _ in 0..7 {
            let start = std::time::Instant::now();
            setup.ok(name, input.clone());
            times.push(start.elapsed());
        }
        times.sort();
        println!("{what}: median {:?} of 7, fastest {:?}", times[3], times[0]);
    };
    time("validate_model, for comparison", "validate_model", json!({ "path": "large.json" }));
    time("describe_model", "describe_model", json!({ "path": "large.json" }));
    let kinds = [
        ("neighbours", json!({ "kind": "neighbours", "name": at(200, &middle) })),
        ("upstream", json!({ "kind": "upstream", "name": at(200, &last) })),
        ("downstream", json!({ "kind": "downstream", "name": at(200, &first) })),
        ("paths", json!({ "kind": "paths", "from": at(200, &first), "to": at(200, &last) })),
        ("paths by process_time, smallest", json!({ "kind": "paths", "from": at(200, &first), "to": at(200, &last), "weight": { "attribute": "process_time" } })),
        ("paths by process_time, largest", json!({ "kind": "paths", "from": at(200, &first), "to": at(200, &last), "weight": { "attribute": "process_time" }, "order": "largest" })),
        ("subgraph_around", json!({ "kind": "subgraph_around", "name": at(200, &middle), "steps": 3 })),
        ("find_instances", json!({ "kind": "find_instances", "class": "Station", "conditions": [{ "attribute": "process_time", "op": ">", "value": "0:30" }] })),
        ("class_tree", json!({ "kind": "class_tree", "instances": true })),
    ];
    for (kind, query) in kinds {
        time(&format!("query_model {kind}"), "query_model", json!({ "path": "large.json", "query": query }));
    }
}
