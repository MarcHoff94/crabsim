//! What the user agreed, kept from agents (ADR 0030): an agent may propose expectations and
//! requests, change its own proposals, question an accepted expectation and mark a request built,
//! but never accept anything, nor change or remove what the user accepted, nor change how an
//! accepted expectation is checked. Every operation that changes a model file asks here before it
//! writes, comparing the file before and after, so no way round it, such as writing the whole
//! file, adopting one written outside or deleting it, lets an agent move the goalposts.

use crate::error::{ErrorKind, OperationError};
use crate::Catalogue;
use crabsim_model::expect::{Aggregate, Check, CheckSubject};
use crabsim_model::{Expectation, Model};

/// Refuses an agent a change from `before` to `after`, each a model file's text, `None` for no
/// file, that touches what the user agreed. The user may change anything.
pub(crate) fn guard(catalogue: &Catalogue, path: &str, before: Option<&str>, after: Option<&str>) -> Result<(), OperationError> {
    if catalogue.source().is_user() {
        return Ok(());
    }
    let read = |text: Option<&str>| text.map(|text| crabsim_model::load_what_can_be_read(text).0).unwrap_or_default();
    let touched = touches(&read(before), &read(after));
    if touched.is_empty() {
        return Ok(());
    }
    Err(OperationError::new(
        ErrorKind::Agreed,
        format!(
            "this would change what the user agreed in `{path}`: it {}. Only the user does that. Question an accepted expectation with question_expectation, saying why, and tell your user",
            touched.join(", ")
        ),
    ))
}

/// What a change from `before` to `after` does to what the user agreed, each as a phrase.
pub(crate) fn touches(before: &Model, after: &Model) -> Vec<String> {
    let mut touched = Vec::new();
    for (index, was) in before.expectations.iter().enumerate().filter(|(_, expectation)| expectation.state.in_force()) {
        let id = &was.id;
        let Some(at) = after.expectations.iter().position(|expectation| &expectation.id == id) else {
            touched.push(format!("removes the expectation `{id}`, which the user accepted"));
            continue;
        };
        let now = &after.expectations[at];
        if !now.state.in_force() {
            touched.push(format!("takes back the user's acceptance of `{id}`"));
        } else if was.state == crabsim_model::Agreement::Questioned && now.state == crabsim_model::Agreement::Accepted {
            touched.push(format!("accepts `{id}` again, answering its question"));
        }
        if (&was.stage, &was.assumptions, &was.source) != (&now.stage, &now.assumptions, &now.source) || !same_check(before, was, after, now) {
            touched.push(format!("changes the expectation `{id}`, which the user accepted"));
        } else if before.settings_of(index) != after.settings_of(at) {
            touched.push(format!("changes how `{id}` is checked"));
        }
    }
    for now in after.expectations.iter().filter(|expectation| expectation.state.in_force()) {
        if !before.expectations.iter().any(|was| was.id == now.id && was.state.in_force()) {
            touched.push(format!("accepts the expectation `{}`", now.id));
        }
    }
    for was in before.requests.iter().filter(|request| request.accepted) {
        let id = &was.id;
        match after.request(id) {
            None => touched.push(format!("removes the request `{id}`, which the user accepted")),
            Some(now) if !now.accepted => touched.push(format!("takes back the user's acceptance of `{id}`")),
            Some(now) if (&was.words, &was.restatement, &was.expectations) != (&now.words, &now.restatement, &now.expectations) => {
                touched.push(format!("changes the request `{id}`, which the user accepted"))
            }
            Some(_) => {}
        }
    }
    for now in after.requests.iter().filter(|request| request.accepted) {
        if !before.request(&now.id).is_some_and(|was| was.accepted) {
            touched.push(format!("accepts the request `{}`", now.id));
        }
    }
    touched
}

/// Whether two expectations check the same thing of the same subject: the same line but for its
/// subject's name, which may follow a rename, an instance being the same when it keeps its stream
/// (ADR 0012), and a class when its old name is gone and its new one new.
fn same_check(before: &Model, was: &Expectation, after: &Model, now: &Expectation) -> bool {
    let (Ok(old), Ok(new)) = (Check::parse(&was.text), Check::parse(&now.text)) else {
        return was.text == now.text;
    };
    let unnamed = |check: &Check| check.with_subject(CheckSubject::Model);
    if unnamed(&old) != unnamed(&new) {
        return false;
    }
    match (&old.subject, &new.subject) {
        (CheckSubject::Model, CheckSubject::Model) => true,
        (CheckSubject::Named(old_name), CheckSubject::Named(new_name)) if old.aggregate == Aggregate::One => {
            let stream = |model: &Model, name: &str| model.instances.iter().find(|instance| instance.name == name).and_then(|instance| instance.stream);
            let old_stream = stream(before, old_name);
            old_stream.is_some() && old_stream == stream(after, new_name)
        }
        (CheckSubject::Named(old_name), CheckSubject::Named(new_name)) => {
            old_name == new_name || (after.class(old_name).is_none() && before.class(new_name).is_none() && after.class(new_name).is_some())
        }
        _ => false,
    }
}
