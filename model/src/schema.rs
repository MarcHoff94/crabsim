//! The published JSON Schema of a model file, generated from the types below. They mirror the
//! file's shape for `schemars`, while `read.rs` reads files by hand so it can report every problem
//! at once. Tests keep the two from drifting apart: every file the reader accepts must pass the
//! schema, the attributes must match, and the committed schema file must be the one generated here.
//!
//! The schema checks a file's shape, so an editor can point out mistakes as they are typed. The
//! reader's checks go further: names that must exist, classes going round in a circle, seconds
//! below 60, a Source that would create MUs forever. The operation catalogue can reuse these types
//! for its own input schemas, so the two never disagree.

use schemars::JsonSchema;
use serde_json::Value;

/// A time, written as crabsim's UI shows it: seconds, minutes and seconds, or hours, minutes and
/// seconds, with days in front and a fraction of a second at the end. Such as "90", "1:30",
/// "8:00:00", "2d 06:00:00", "3d" or "0:00:12.5".
#[derive(JsonSchema)]
#[schemars(extend("pattern" = r"^\s*(\d+\s*d(\s*((\d+:)?\d+:)?\d+(\.\d+)?)?|((\d+:)?\d+:)?\d+(\.\d+)?)\s*$"))]
pub struct Time(pub String);

/// A name: not empty, and with no spaces at either end.
#[derive(JsonSchema)]
#[schemars(extend("pattern" = r"^\S(.*\S)?$"))]
pub struct Name(pub String);

/// A crabsim model file. It holds the model only: a run's seed, length and warm-up are not part
/// of it.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "crabsim model file")]
pub struct ModelFile {
    /// Where this schema is, for editors. crabsim ignores it, keeps it when saving, and never
    /// needs the file it names to exist.
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    /// The format's version, 2. Version 1, with each instance's `successors` instead of a list of
    /// connections, isn't read any more.
    #[schemars(extend("const" = 2))]
    pub format_version: u64,
    /// What the model is, as a whole.
    pub description: Option<String>,
    /// The model's own classes, in any order. Each derives from a built-in class (Source,
    /// Station, Buffer, Drain, Conveyor, or MU for an MU class) or from another class here.
    pub classes: Option<Vec<ClassFile>>,
    /// The material-flow objects, in the order a run initialises them.
    pub instances: Vec<InstanceFile>,
    /// The ways MUs pass from one instance to another, in any order: each says its own place in
    /// both orders it belongs to, so sorting the list changes nothing.
    pub connections: Option<Vec<ConnectionFile>>,
    /// The model's hooks around crabsim's own Init and Reset, each running user code (ADR 0029).
    pub hooks: Option<ModelHooksFile>,
    /// How each stage's checks run (ADR 0030), where they differ from the defaults: verification
    /// a day after no warm-up, one deterministic observation; validation a day, ten observations.
    pub stages: Option<StagesFile>,
    /// What the user wants from the model, in their own words, each with an agent's restatement
    /// and the expectations that check it (ADR 0030).
    pub requests: Option<Vec<RequestFile>>,
    /// What the model must achieve: checks crabsim judges on its own after a run (ADR 0030).
    pub expectations: Option<Vec<ExpectationFile>>,
}

/// Each stage's run settings.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StagesFile {
    pub verification: Option<RunSettingsFile>,
    pub validation: Option<RunSettingsFile>,
}

/// How a stage's checks run, each part left out taking its default.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunSettingsFile {
    /// How long each observation runs after the warm-up, such as "1d" or "8:00:00".
    pub length: Option<Time>,
    /// Simulated time at the start of each observation that statistics leave out.
    pub warm_up: Option<Time>,
    /// How many observations, from 1 to 10,000.
    #[schemars(range(min = 1, max = 10_000))]
    pub observations: Option<u32>,
}

/// A request: what the user wants from the model.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestFile {
    /// Unique among the model's requests, with no spaces, such as `r1`.
    #[schemars(extend("pattern" = r"^\S+$"))]
    pub id: String,
    /// The user's own words, exactly.
    pub words: String,
    /// An agent's restatement in precise terms.
    pub restatement: Option<String>,
    /// The ids of the expectations that check it.
    pub expectations: Option<Vec<String>>,
    /// The user accepted its restatement and expectations, which only the user does. False if
    /// left out.
    pub accepted: Option<bool>,
    /// `open`, the default, or `built` once whoever builds it says so. Whether it is met is what
    /// crabsim's checks say, never written here.
    pub state: Option<RequestStateFile>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequestStateFile {
    Open,
    Built,
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgreementFile {
    Proposed,
    Accepted,
    Questioned,
}

/// An expectation: a check of the model crabsim judges on its own.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpectationFile {
    /// Unique among the model's expectations, with no spaces, such as `presses-busy`.
    #[schemars(extend("pattern" = r"^\S+$"))]
    pub id: String,
    /// The check as one line: `[every|mean] <instance, class or model>.<figure> <now|since start|
    /// last <time>|always> <comparison> <value>`, such as `every Press.working_share since start
    /// >= 60%`, `Buffer X.mus always <= 20` or `model.throughput_per_hour last 8h >= 400/h`. The
    /// figures are the results store's, which list_figures gives with their units and scopes.
    pub text: String,
    /// The stage it belongs to: `verification`, the default, checked in a deterministic run, or
    /// `validation`, checked in runs with randomness.
    pub stage: Option<StageFile>,
    /// What it takes for granted, such as "two shifts a day".
    pub assumptions: Option<String>,
    /// Where it comes from: a SysML element, a document or a person.
    pub source: Option<String>,
    /// `proposed`, the default, by an agent and not accepted yet; `accepted` by the user, or
    /// written by the user, which only the user changes; or `questioned` by an agent, still in
    /// force until the user answers.
    pub state: Option<AgreementFile>,
    /// Why an agent questions it: only while it is questioned.
    pub question: Option<String>,
    /// How long its check runs after the warm-up, where it differs from its stage's.
    pub length: Option<Time>,
    /// Its check's warm-up, where it differs from its stage's.
    pub warm_up: Option<Time>,
    /// Its check's observations, where they differ from its stage's.
    #[schemars(range(min = 1, max = 10_000))]
    pub observations: Option<u32>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StageFile {
    Verification,
    Validation,
}

/// The model's hooks: before Init, with objects in their default state and nothing queued; after
/// Init, every object's Init done; before Reset, which may only read and log; and after Reset,
/// objects back in their default state and the code's memory fresh.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelHooksFile {
    pub before_init: Option<LifecycleHookFile>,
    pub after_init: Option<LifecycleHookFile>,
    pub before_reset: Option<LifecycleHookFile>,
    pub after_reset: Option<LifecycleHookFile>,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleHookFile {
    pub code: CodeCallFile,
}

/// A call of user code: a module's function and its parameters' values.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeCallFile {
    /// `<module>.<function>`, the module being `code/<module>.wasm` in the models folder.
    #[schemars(regex(pattern = r"^[A-Za-z0-9_-]+\.[A-Za-z0-9_]+$"))]
    pub function: String,
    /// Each parameter's value by name, as the function declares them: a number, true or false, a
    /// time as text such as "1:30", or an instance's name.
    pub with: Option<serde_json::Map<String, serde_json::Value>>,
}

/// A hook on entry or on exit: user code run at each trigger point it names.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeHookFile {
    pub code: CodeCallFile,
    /// Where along the MU it runs, each once: its front, its booking point or its rear. The
    /// booking point alone by default.
    #[schemars(length(min = 1))]
    pub at: Option<Vec<TriggerPointFile>>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TriggerPointFile {
    Front,
    BookingPoint,
    Rear,
}

/// A distribution user code chooses: one successor, which the MU then waits for alone, hold, or
/// its default strategy.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeDistributionFile {
    pub code: CodeCallFile,
    /// The strategy that chooses when the code answers default. Round robin when left out.
    pub default: Option<CodeDefaultFile>,
    /// Asks the code as the MU comes in, at this trigger point, rather than when it wants to
    /// leave, at the user's own risk. Not for a CrossConveyor.
    pub asked_on_entry: Option<TriggerPointFile>,
}

/// Round robin, weighted or by destination.
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum CodeDefaultFile {
    RoundRobin(RoundRobinFile),
    Weighted(WeightedFile),
    ByDestination(ByDestinationFile),
}

/// A connection from one instance to another. Moving along it takes no time.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionFile {
    /// The instance MUs leave, which can't be a Drain.
    pub from: String,
    /// The instance MUs enter, which can't be a Source or `from` itself.
    pub to: String,
    /// Its place among the connections out of `from`, the order MUs are offered round-robin:
    /// 1 to n with no gaps or repeats.
    #[schemars(range(min = 1))]
    pub exit: u64,
    /// Its place among the connections into `to`, 1 to n with no gaps or repeats. Nothing reads
    /// it yet, so it doesn't change results until an entrance rule does.
    #[schemars(range(min = 1))]
    pub entrance: u64,
    /// What the connection is, such as "via the lift".
    pub description: Option<String>,
    /// The side of `from` it leaves by: given when `from` is a cross conveyor, and only then. A
    /// side is an entry side or an exit side, never both, and has one connection.
    pub from_side: Option<SideFile>,
    /// The side of `to` it comes in by: given when `to` is a cross conveyor, and only then.
    pub to_side: Option<SideFile>,
}

/// One of a cross conveyor's four sides: front, where its own x axis points, back, left, 90°
/// counter-clockwise from the front, and right.
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SideFile {
    Front,
    Back,
    Left,
    Right,
}

/// A cross conveyor's exit sides, each with the destinations it takes: instances, or classes
/// standing for any of their instances.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExitsFile {
    pub front: Option<Vec<String>>,
    pub back: Option<Vec<String>>,
    pub left: Option<Vec<String>>,
    pub right: Option<Vec<String>>,
}

/// A class of the model's own, or, named like a built-in class and with no parent, this model's
/// own values for that built-in class, which every class and instance built on it inherits.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClassFile {
    /// Unique among classes. Source, Station, Buffer, Drain, Conveyor or MU makes it this model's
    /// own values for that built-in class.
    pub name: Name,
    /// The class it derives from: Source, Station, Buffer, Drain, Conveyor, MU or another class in
    /// the file.
    /// Left out only on a class named like a built-in class, and required on every other.
    pub parent: Option<String>,
    /// Only the values this class changes from its parent.
    pub attributes: Option<AttributesFile>,
}

/// One material-flow object.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstanceFile {
    /// Unique among instances.
    pub name: Name,
    /// Source, Station, Buffer, Drain, Conveyor or a class in the file derived from one: not an MU
    /// class.
    pub class: String,
    /// Its stream: with a run's seed, it gives the instance's random numbers, so runs of two
    /// variants with the same seed draw the same numbers for it. Unique in the model. crabsim
    /// gives one when the instance is placed: changing it ends pairing with every earlier run.
    pub stream: Option<u64>,
    /// The centre of its footprint, in metres. Visual only.
    pub position: Option<PositionFile>,
    /// Only the values set on this instance. Anything else comes from its class.
    pub attributes: Option<AttributesFile>,
}

/// A position in metres.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PositionFile {
    pub x: f64,
    pub y: f64,
    /// Height above the floor, 0 if left out.
    pub z: Option<f64>,
}

/// The values a class or instance sets. Each applies to some built-in classes only, and anything
/// left out is inherited.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributesFile {
    /// Source: the time between creating an MU and offering it on. 0 by default, which creates
    /// the next MU as soon as one leaves.
    pub interval: Option<Time>,
    /// Station: the time it works on each MU. 0 by default, a dummy Station that takes no time.
    pub process_time: Option<Time>,
    /// Any object: random failures. Left out, it never fails.
    pub failures: Option<FailuresValue>,
    /// Station, Buffer: how many MUs it holds from the start of a run, at most its capacity, in
    /// the order they count as having entered. 0 by default.
    pub starting_mus: Option<u64>,
    /// Station, Buffer: how many MUs it holds. A Station's is a whole number from 1 to 10,000, and
    /// 1 by default: one machine working on that many MUs at once. A Buffer's is unlimited by
    /// default.
    pub capacity: Option<CapacityFile>,
    /// Station: when it begins work on its MUs. Individual by default.
    pub start: Option<StartFile>,
    /// Buffer: how long each MU stays before it may leave, counted from its own entry. 0 by
    /// default.
    pub dwelling_time: Option<Time>,
    /// Buffer: which of the MUs whose dwelling time is over it passes on next. First in, first
    /// out by default.
    pub exit_strategy: Option<ExitStrategyFile>,
    /// Buffer: how many MUs each of its fill histogram's 64 bars covers, from 1. Left out, it is
    /// automatic: one each, doubling whenever more MUs come than the bars reach. Set, the last bar
    /// collects everything beyond, such as 315 or more with a width of 5.
    #[schemars(range(min = 1))]
    pub histogram_bar_width: Option<u64>,
    /// Buffer: whether an interruption freezes the dwelling time of the MUs dwelling in it, so they
    /// become ready later by its length. False by default: their dwelling time runs on.
    pub stop_freezes_dwelling: Option<bool>,
    /// Any object: its size in metres and rotation in degrees. Visual only.
    pub dimensions: Option<DimensionsFile>,
    /// Any object: free text saying what it is. Visual only.
    pub description: Option<String>,
    /// Any object: the model's own note on what a state means for it, by state, shown beside
    /// crabsim's definition of the state. Only states its built-in class can be in, each note not
    /// empty. Inherited as a whole. Visual only.
    pub state_notes: Option<StateNotesFile>,
    /// Source, Station, Buffer, Conveyor: which successor receives the MU it passes on (ADR 0016).
    /// Round robin by default. Inherited as a whole.
    pub distribution: Option<DistributionFile>,
    /// Source: the MU class whose MUs it makes, `MU` or a class derived from it. `MU` by default.
    pub mu_class: Option<String>,
    /// Source: its own values for the MUs it makes, changing its MU class's. Inherited as a whole.
    pub mu_values: Option<MuValuesFile>,
    /// MU class: its footprint's length along the way it travels, in metres. 0.6 by default.
    /// CrossConveyor: its length along its own x axis, from its back to its front, 0.6 by default.
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub length: Option<f64>,
    /// MU class: its footprint's width across the way it travels, in metres, 0.4 by default.
    /// Conveyor: how wide it is, in metres, 0.6 by default: an MU wider than this doesn't fit.
    /// CrossConveyor: its width from its right side to its left, 0.6 by default.
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub width: Option<f64>,
    /// MU class: where its booking point is, as a share of its length from its front, 0 at the
    /// front and 1 at the rear. Where it is decides when it passes from one object to the next.
    /// 0.5 by default, its centre.
    #[schemars(range(min = 0, max = 1))]
    pub booking_point: Option<f64>,
    /// Conveyor: the line its MUs follow (ADR 0024), as segments from its position, heading along
    /// its rotation, each starting where the one before ends. Lengths are in plan, and the
    /// conveyor's length counts each in 3D, rise included. One straight of 5 m by default.
    #[schemars(length(min = 1))]
    pub path: Option<Vec<SegmentFile>>,
    /// Conveyor, CrossConveyor: how fast it moves its MUs, in metres per second, above 0. 0.5 by
    /// default.
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub speed: Option<f64>,
    /// Conveyor: the least space between two MUs on it, in metres, 0 or more. 0 by default.
    #[schemars(range(min = 0))]
    pub gap: Option<f64>,
    /// Conveyor: whether MUs behind a stopped one close up and stop while the rest still move,
    /// true by default, or every MU stops as a whole when the first can't leave and they all start
    /// again together.
    pub accumulating: Option<bool>,
    /// Conveyor: which way its MUs travel along its path, "forward", from its first segment to its
    /// last, the default, or "backward". Its start, where MUs enter, is then the path's last point.
    pub direction: Option<DirectionFile>,
    /// Conveyor: the tightest bend its path may have, as a radius in metres. Unset by default:
    /// then only half its width, inside which its inner edge would fold.
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub min_radius: Option<f64>,
    /// Conveyor: the steepest its path may climb or fall, in degrees. 30 by default.
    #[schemars(extend("exclusiveMinimum" = 0, "exclusiveMaximum" = 90))]
    pub max_incline: Option<f64>,
    /// MU class: the steps its MUs follow (ADR 0026), each a destination and an optional process
    /// time. An MU entering a Station its next step names is worked on for the step's time, or
    /// the Station's own without one, and moves on to its next step. None by default. Inherited
    /// as a whole.
    pub work_plan: Option<Vec<WorkStepFile>>,
    /// Station: where every MU it works on goes before carrying on with its plan, such as Rework:
    /// an instance, or a class standing for any of its instances (ADR 0025). None by default.
    pub detour: Option<String>,
    /// CrossConveyor: how long an MU turning there stands at its centre, such as a lift raising
    /// it onto cross rollers. 2 s by default.
    pub switch_time: Option<Time>,
    /// CrossConveyor: how long it takes no MU after one that turned has left, such as its lift
    /// going back down. 0 by default.
    pub release_time: Option<Time>,
    /// CrossConveyor: each exit side's destinations. An MU whose next destination one of them
    /// stands for leaves by that side, turning at the centre, unless the side straight across
    /// takes it too; every other MU goes straight through. None by default. Inherited as a whole.
    pub exits: Option<ExitsFile>,
    /// Any type: user code run as an MU comes in (ADR 0029). None by default. Inherited as a
    /// whole.
    pub on_entry: Option<CodeHookFile>,
    /// Any type but Drain: user code run as an MU leaves, after it has moved on. None by
    /// default. Inherited as a whole.
    pub on_exit: Option<CodeHookFile>,
}

/// One step of a work plan.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkStepFile {
    /// Where the MU goes: an instance, or a class standing for any of its instances (ADR 0025).
    pub at: String,
    /// How long a Station there works on it. Only for a step that can be a Station. Without it,
    /// the Station's own process time.
    pub process_time: Option<Time>,
}

/// One piece of a conveyor's path: exactly one of a straight, an arc, a transition curve and a
/// helix. Angles and turns are positive to the left, the way rotation turns, and negative to the
/// right. A `rise` is the change in height over the segment in metres, negative falling.
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum SegmentFile {
    Straight(StraightSegmentFile),
    Arc(ArcSegmentFile),
    Transition(TransitionSegmentFile),
    Helix(HelixSegmentFile),
}

/// Metres, above 0.
#[derive(JsonSchema)]
#[schemars(extend("exclusiveMinimum" = 0))]
pub struct Metres(pub f64);

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StraightSegmentFile {
    /// Straight on: its length in metres, or its length and rise.
    pub straight: StraightFile,
}

#[derive(JsonSchema)]
#[serde(untagged)]
pub enum StraightFile {
    Length(Metres),
    Rising(RisingStraightFile),
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RisingStraightFile {
    /// Its length in plan, in metres.
    pub length: Metres,
    pub rise: Option<f64>,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArcSegmentFile {
    pub arc: ArcFile,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArcFile {
    /// Its radius in plan, in metres, more than half the conveyor's width.
    pub radius: Metres,
    /// How far it turns, in degrees, not 0: positive to the left.
    pub angle: f64,
    pub rise: Option<f64>,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransitionSegmentFile {
    /// A clothoid, its bend changing evenly from the bend the segment before ended with, or
    /// straight for the first, to `to_radius`.
    pub transition: TransitionFile,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransitionFile {
    /// Its length in plan, in metres.
    pub length: Metres,
    /// The radius it eases to in metres, positive to the left, or "straight".
    pub to_radius: ToRadiusFile,
    pub rise: Option<f64>,
}

#[derive(JsonSchema)]
#[serde(untagged)]
pub enum ToRadiusFile {
    Radius(f64),
    Straight(StraightWordFile),
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StraightWordFile {
    Straight,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HelixSegmentFile {
    pub helix: HelixFile,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HelixFile {
    /// Its radius in plan, in metres.
    pub radius: Metres,
    /// How many times it turns, not 0: positive to the left.
    pub turns: f64,
    /// How far it climbs over its turns, in metres, not 0: negative falling. A flat turn is an
    /// arc.
    pub rise: f64,
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DirectionFile {
    /// From the path's first segment to its last.
    Forward,
    /// From the path's last segment to its first.
    Backward,
}

/// A Source's own values for the MUs it makes, each changing its MU class's when set.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MuValuesFile {
    /// In metres, above 0.
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub length: Option<f64>,
    /// In metres, above 0.
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub width: Option<f64>,
    /// A share of its length from its front, from 0 to 1.
    #[schemars(range(min = 0, max = 1))]
    pub booking_point: Option<f64>,
    /// Its own work plan for the MUs it makes, in place of its MU class's (ADR 0026).
    pub work_plan: Option<Vec<WorkStepFile>>,
}

/// A whole number from 1, or `"unlimited"`: it then takes every MU and grows as far as it's
/// filled. -1 is read as unlimited too, and saved as `"unlimited"`.
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum CapacityFile {
    Limited(FromOne),
    Unlimited(UnlimitedFile),
    MinusOne(MinusOne),
}

#[derive(JsonSchema)]
#[schemars(extend("minimum" = 1))]
pub struct FromOne(pub u64);

#[derive(JsonSchema)]
#[schemars(extend("const" = -1))]
pub struct MinusOne(pub i64);

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnlimitedFile {
    Unlimited,
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StartFile {
    /// Each MU starts on entry, with its own process time.
    Individual,
    /// Work begins when the Station is full, and its entrance stays closed until it is empty.
    Batch,
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExitStrategyFile {
    /// The MU that came in first.
    FirstInFirstOut,
    /// The MU that came in last, as from a stack.
    LastInFirstOut,
}

/// `"round_robin"`, each successor in turn, weighted: the best-ranked successor that can take
/// the MU, those ranking equal taking turns, or by destination: only the successors from which
/// the MU's next destination can be reached, and among them `then`.
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum DistributionFile {
    RoundRobin(RoundRobinFile),
    Weighted(WeightedFile),
    ByDestination(ByDestinationFile),
    Code(CodeDistributionFile),
    Rules(RulesDistributionFile),
}

/// A distribution by when/then rules (ADR 0030), read in order, the first that holds deciding:
/// `when <conditions> then <action>`, or `otherwise <action>` for one that always holds.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RulesDistributionFile {
    /// The lines, such as `when class = Frame then smallest working_share last 30min of Line A, Line B among those free`, `when time of day >= 22:00 then to Night shift` or `otherwise default`. Conditions: `class = <MU class>`, `destination = <destination>`, `time of day <comparison> <hh:mm>`, `<instance>.state now = <state>`, or `<instance>.<figure> <now|since start|last <time>> <comparison> <value or figure>`, joined by `and` and `or`. Actions: `to <successor>`, `smallest` or `largest <figure> <scope> [of <successors>] [among those free|and wait]`, `round robin [among <successors>]`, `weighted among <successors>` or `default`.
    #[schemars(length(min = 1))]
    pub rules: Vec<String>,
    /// The strategy `default` means, and that decides when no line holds. Round robin when left
    /// out.
    pub default: Option<CodeDefaultFile>,
}

/// The strategy that chooses among the successors that reach the MU's destination.
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum ThenFile {
    RoundRobin(RoundRobinFile),
    Weighted(WeightedFile),
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ByDestinationFile {
    pub by_destination: ByDestinationFields,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ByDestinationFields {
    /// Round robin or weighted. Round robin when left out.
    pub then: Option<ThenFile>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoundRobinFile {
    RoundRobin,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeightedFile {
    pub weighted: WeightedFields,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeightedFields {
    /// What each object is weighed by, resolved through inheritance. An object without it weighs
    /// 0, such as a Drain for process_time.
    pub by: WeighByFile,
    /// Which totals rank first: smallest by default.
    pub order: Option<OrderFile>,
    /// What a successor is ranked by: `route`, its best route onward to a Drain with itself
    /// included, by default, or `successor`, its own weight only.
    pub rank: Option<RankFile>,
    /// The Drains a route may end in. Left out, every Drain counts. A name that isn't a Drain is
    /// ignored, and validate_model warns about it.
    #[schemars(length(min = 1))]
    pub drains: Option<Vec<String>>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WeighByFile {
    ProcessTime,
    Interval,
    StartingMus,
    Capacity,
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrderFile {
    Smallest,
    Largest,
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RankFile {
    Route,
    Successor,
}

/// A note per state, each left out if there is nothing to say.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StateNotesFile {
    /// Source, Station.
    #[schemars(length(min = 1))]
    pub busy: Option<String>,
    /// Buffer: it has MUs in it, none of which is kept from leaving.
    #[schemars(length(min = 1))]
    pub holding: Option<String>,
    /// Source, Station, Buffer.
    #[schemars(length(min = 1))]
    pub blocked: Option<String>,
    /// Station, Buffer, Drain.
    #[schemars(length(min = 1))]
    pub idle: Option<String>,
    /// Any object.
    #[schemars(length(min = 1))]
    pub failed: Option<String>,
    /// Any object: interrupted by hand.
    #[schemars(length(min = 1))]
    pub stopped: Option<String>,
    /// Any object: Stopped's name before ADR 0020, still read.
    #[schemars(length(min = 1))]
    pub forced_failure: Option<String>,
    /// Any object.
    #[schemars(length(min = 1))]
    pub paused: Option<String>,
    /// Station, Buffer, Drain: a place reserved for an MU a conveyor is handing over.
    #[schemars(length(min = 1))]
    pub receiving: Option<String>,
}

/// Failure profiles: a list of named ones, which are independent of each other, or, as older files
/// have it, a single unnamed one, read as a list of one named "Failures".
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum FailuresValue {
    List(Vec<FailureProfileFile>),
    One(FailuresFile),
}

/// One named failure profile.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FailureProfileFile {
    /// Unique among the object's profiles.
    #[schemars(length(min = 1))]
    pub name: String,
    /// Its random-number stream within the object, 0 or more, unique there. Without one, its place
    /// in the list from 0.
    pub stream: Option<u64>,
    /// What counts down to its next failure. Simulation time by default.
    pub time_basis: Option<TimeBasisFile>,
    /// The share of time available, above 0 and below 1.
    #[schemars(extend("exclusiveMinimum" = 0, "exclusiveMaximum" = 1))]
    pub availability: f64,
    pub repair_time: RepairTimeFile,
}

/// The kind of time that counts down to a failure profile's next failure.
#[derive(JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TimeBasisFile {
    /// All time counts, whatever the object is doing.
    Simulation,
    /// All time but paused, idle and blocked included.
    Working,
    /// Only time busy, once however many MUs are in work: a Station or Source only.
    Processing,
}

/// Random failures. The time between failures follows from the availability and the repair time.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FailuresFile {
    /// The share of time available, above 0 and below 1.
    #[schemars(extend("exclusiveMinimum" = 0, "exclusiveMaximum" = 1))]
    pub availability: f64,
    pub repair_time: RepairTimeFile,
}

/// How long a repair takes: exactly one of these.
#[derive(JsonSchema)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum RepairTimeFile {
    /// Always the same time, above 0.
    Constant(Time),
    /// Evenly spread between `min` and `max`.
    Uniform { min: Time, max: Time },
    /// Normally distributed with this mean and standard deviation.
    Normal { mean: Time, std: Time },
    /// Erlang distributed with this mean and shape `k`.
    Erlang {
        mean: Time,
        #[schemars(range(min = 1))]
        k: u64,
    },
}

/// An object's size and rotation.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DimensionsFile {
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub length: f64,
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub width: f64,
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub height: f64,
    /// Around the vertical axis, 0 if left out.
    pub rotation_degrees: Option<f64>,
}

/// The JSON Schema of a model file.
pub fn schema() -> Value {
    let generator = schemars::generate::SchemaSettings::draft2020_12().into_generator();
    let mut schema = serde_json::to_value(generator.into_root_schema_for::<ModelFile>()).expect("a schema always serialises");
    tidy(&mut schema);
    schema
}

/// `schemars` lets an `Option` field be `null`, but a file leaves the field out instead, and
/// crabsim refuses `null`. So this takes `null` out of every choice of types. It also joins the
/// lines of descriptions, which come from wrapped doc comments.
fn tidy(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            if let Some(Value::Array(types)) = fields.get_mut("type") {
                types.retain(|kind| kind != "null");
                if types.len() == 1 {
                    let only = types.remove(0);
                    fields.insert("type".into(), only);
                }
            }
            if let Some(Value::Array(choices)) = fields.get_mut("anyOf") {
                choices.retain(|choice| choice.get("type").is_none_or(|kind| kind != "null"));
                if choices.len() == 1 {
                    let Value::Object(only) = choices.remove(0) else { unreachable!("a choice is a schema") };
                    fields.remove("anyOf");
                    fields.extend(only);
                }
            }
            if let Some(Value::String(description)) = fields.get_mut("description") {
                *description = description.replace('\n', " ");
            }
            fields.values_mut().for_each(tidy);
        }
        Value::Array(items) => items.iter_mut().for_each(tidy),
        _ => {}
    }
}

/// The JSON Schema as the committed file `models/model-file.schema.json` holds it.
pub fn schema_text() -> String {
    let mut text = serde_json::to_string_pretty(&schema()).expect("a schema always serialises");
    text.push('\n');
    text
}
