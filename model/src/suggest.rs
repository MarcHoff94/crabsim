//! "Did you mean": the known name closest to a mistyped one, for validation messages, so an agent
//! or a person fixes a typo such as `proces_time` in one pass.

/// The candidate closest to `name` by edit distance, if it is at most 2 edits away and no more
/// than a third of `name` changed, so a short name isn't matched to anything at all. The first
/// candidate wins a tie, so the answer never depends on anything but the input.
pub(crate) fn closest<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let length = name.chars().count();
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        if candidate == name {
            continue;
        }
        let distance = edit_distance(name, candidate);
        if distance <= 2 && distance * 3 <= length && best.is_none_or(|(shortest, _)| distance < shortest) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// ": did you mean `x`?" for the closest candidate, or nothing.
pub(crate) fn did_you_mean<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> String {
    closest(name, candidates).map_or(String::new(), |candidate| format!(": did you mean `{candidate}`?"))
}

/// The edit distance by characters, counting an insertion, a deletion, a substitution or two
/// neighbours swapped as one edit each (optimal string alignment), since swapped letters, as in
/// `Statoin`, are the commonest typo.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b.len() {
        d[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_closest_name_within_two_edits() {
        assert_eq!(closest("proces_time", ["interval", "process_time"]), Some("process_time"));
        assert_eq!(closest("Statoin", ["Source", "Station", "Drain"]), Some("Station"));
        assert_eq!(closest("Otu", ["In", "Out"]), Some("Out"));
        assert_eq!(closest("Machine", ["Source", "Station", "Buffer", "Drain"]), None);
        assert_eq!(closest("ab", ["xy", "ac"]), None, "a third at most of a short name");
        assert_eq!(closest("Mill 3", ["Mill 1", "Mill 2"]), Some("Mill 1"), "the first wins a tie");
    }
}
