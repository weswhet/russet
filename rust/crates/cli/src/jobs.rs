//! Running several recipes at once with `--jobs` or the `RussetJobs`
//! preference.
//!
//! Recipes that share a cache directory form a group, and a group's recipes
//! run one after another, in list order, on one worker. A barrier recipe,
//! such as one that rebuilds Munki catalogs, waits for every recipe before
//! it in the list, and every recipe after it waits for it. Results are kept
//! by list position, so reports don't depend on which recipe finishes first.
use plist::{Dictionary, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
    thread,
};

/// The preference that sets how many recipes run at once.
pub const PREFERENCE: &str = "RussetJobs";

/// Workers get the stack size of the main thread, which recipes ran on
/// before they could run in parallel.
const STACK_SIZE: usize = 8 << 20;

/// Reads a job count: a whole number, where 0 means one per CPU.
pub fn parse(text: &str) -> Option<usize> {
    let text = text.trim();
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// The `RussetJobs` preference from `-k`, the recipe list, or an
/// `AUTOPKG_RussetJobs` variable, and then from the preferences.
pub fn preference(keys: &Dictionary, preferences: &Dictionary) -> Result<Option<usize>, String> {
    let invalid = || format!("{PREFERENCE} must be a whole number of 0 or more");
    match keys.get(PREFERENCE).or_else(|| preferences.get(PREFERENCE)) {
        None => Ok(None),
        Some(Value::Integer(n)) => n
            .as_unsigned()
            .and_then(|n| usize::try_from(n).ok())
            .map(Some)
            .ok_or_else(invalid),
        Some(Value::String(s)) => parse(s).map(Some).ok_or_else(invalid),
        Some(_) => Err(invalid()),
    }
}

/// Turns 0 into the number of CPUs.
pub fn workers(requested: usize) -> usize {
    match requested {
        0 => thread::available_parallelism().map_or(1, usize::from),
        n => n,
    }
}

/// A stage's groups of list positions. Stages run one after another; a
/// stage's groups can run at the same time.
pub type Stage = Vec<Vec<usize>>;

/// Splits list positions into stages. A barrier position is a stage of its
/// own. Within a stage, positions are grouped by key, in order of each key's
/// first appearance.
pub fn stages<K: PartialEq>(keys: &[K], barriers: &[bool]) -> Vec<Stage> {
    let mut stages: Vec<Stage> = vec![vec![]];
    for (index, key) in keys.iter().enumerate() {
        if barriers[index] {
            stages.push(vec![vec![index]]);
            stages.push(vec![]);
            continue;
        }
        let stage = stages.last_mut().expect("a stage");
        match stage.iter_mut().find(|group| keys[group[0]] == *key) {
            Some(group) => group.push(index),
            None => stage.push(vec![index]),
        }
    }
    stages.retain(|stage| !stage.is_empty());
    stages
}

/// Runs `work` for every list position, one stage at a time. Up to
/// `workers` threads each take the stage's next group. After each position
/// finishes, `finished` sees every result so far, by list position.
///
/// An error stops workers from starting more recipes, and the error for
/// the earliest position is returned once the running recipes finish. With
/// one worker, everything runs on the calling thread.
pub fn run<T: Send>(
    stages: &[Stage],
    workers: usize,
    work: impl Fn(usize) -> Result<T, String> + Sync,
    finished: impl Fn(&[Option<T>]) + Sync,
) -> Result<Vec<T>, String> {
    let length = stages.iter().flatten().map(Vec::len).sum();
    let results = Mutex::new((0..length).map(|_| None).collect::<Vec<Option<T>>>());
    let failure: Mutex<Option<(usize, String)>> = Mutex::new(None);
    let stopped = AtomicBool::new(false);
    for groups in stages {
        if stopped.load(Ordering::SeqCst) {
            break;
        }
        run_stage(
            groups, workers, &work, &finished, &results, &failure, &stopped,
        )?;
    }
    if let Some((_, error)) = failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
        return Err(error);
    }
    Ok(results
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .into_iter()
        .map(|result| result.expect("every recipe ran"))
        .collect())
}

fn run_stage<T: Send>(
    groups: &[Vec<usize>],
    workers: usize,
    work: &(impl Fn(usize) -> Result<T, String> + Sync),
    finished: &(impl Fn(&[Option<T>]) + Sync),
    results: &Mutex<Vec<Option<T>>>,
    failure: &Mutex<Option<(usize, String)>>,
    stopped: &AtomicBool,
) -> Result<(), String> {
    let next = AtomicUsize::new(0);
    let worker = || {
        while !stopped.load(Ordering::SeqCst) {
            let Some(group) = groups.get(next.fetch_add(1, Ordering::SeqCst)) else {
                break;
            };
            for &index in group {
                if stopped.load(Ordering::SeqCst) {
                    break;
                }
                match work(index) {
                    Ok(value) => {
                        let mut results = results.lock().unwrap_or_else(|e| e.into_inner());
                        results[index] = Some(value);
                        finished(&results);
                    }
                    Err(error) => {
                        stopped.store(true, Ordering::SeqCst);
                        let mut failure = failure.lock().unwrap_or_else(|e| e.into_inner());
                        if failure.as_ref().is_none_or(|(first, _)| index < *first) {
                            *failure = Some((index, error));
                        }
                        break;
                    }
                }
            }
        }
    };
    let workers = workers.clamp(1, groups.len().max(1));
    if workers == 1 {
        worker();
    } else {
        thread::scope(|scope| {
            for number in 1..=workers {
                thread::Builder::new()
                    .name(format!("recipe-worker-{number}"))
                    .stack_size(STACK_SIZE)
                    .spawn_scoped(scope, worker)
                    .map_err(|e| format!("Can't start a recipe worker: {e}"))?;
            }
            Ok::<_, String>(())
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn counts_are_whole_numbers() {
        assert_eq!(parse("4"), Some(4));
        assert_eq!(parse(" 0 "), Some(0));
        for invalid in ["", "-1", "+2", "two", "1.5", "99999999999999999999999"] {
            assert_eq!(parse(invalid), None, "{invalid}");
        }
        assert!(workers(0) >= 1);
        assert_eq!(workers(3), 3);
    }

    #[test]
    fn keys_override_the_preference() {
        let prefs = Dictionary::from_iter([(PREFERENCE, Value::from(2))]);
        assert_eq!(preference(&Dictionary::new(), &prefs), Ok(Some(2)));
        let keys = Dictionary::from_iter([(PREFERENCE, Value::from("3"))]);
        assert_eq!(preference(&keys, &prefs), Ok(Some(3)));
        assert_eq!(preference(&Dictionary::new(), &Dictionary::new()), Ok(None));
        for invalid in [Value::from(-1), Value::from("many"), Value::Boolean(true)] {
            let prefs = Dictionary::from_iter([(PREFERENCE, invalid)]);
            assert_eq!(
                preference(&Dictionary::new(), &prefs),
                Err("RussetJobs must be a whole number of 0 or more".into())
            );
        }
    }

    fn groups<K: PartialEq>(keys: &[K]) -> Vec<Stage> {
        stages(keys, &vec![false; keys.len()])
    }

    #[test]
    fn recipes_sharing_a_key_share_a_group() {
        assert_eq!(
            groups(&["a", "b", "a", "c", "b"]),
            [vec![vec![0, 2], vec![1, 4], vec![3]]]
        );
    }

    #[test]
    fn a_barrier_runs_alone_between_the_recipes_around_it() {
        assert_eq!(
            stages(
                &["a", "b", "c", "a", "d"],
                &[false, false, true, false, false]
            ),
            [
                vec![vec![0], vec![1]],
                vec![vec![2]],
                vec![vec![3], vec![4]]
            ]
        );
        assert_eq!(stages(&["a"], &[true]), [vec![vec![0]]]);
    }

    #[test]
    fn a_barrier_waits_for_every_earlier_recipe() {
        let keys = ["slow", "a", "catalogs", "b"];
        let done = Mutex::new(Vec::new());
        run(
            &stages(&keys, &[false, false, true, false]),
            4,
            |index| {
                if keys[index] == "slow" {
                    thread::sleep(Duration::from_millis(30));
                }
                let mut done = done.lock().unwrap();
                if keys[index] == "catalogs" {
                    assert_eq!(done.len(), 2, "the barrier started early");
                }
                if keys[index] == "b" {
                    assert!(done.contains(&"catalogs"), "a later recipe started early");
                }
                done.push(keys[index]);
                Ok(())
            },
            |_| {},
        )
        .unwrap();
    }

    #[test]
    fn results_keep_list_order_and_groups_never_overlap() {
        let keys = ["slow", "a", "slow", "b", "c"];
        let running = Mutex::new(Vec::new());
        let seen = Mutex::new(Vec::new());
        let results = run(
            &groups(&keys),
            3,
            |index| {
                {
                    let mut running = running.lock().unwrap();
                    assert!(!running.contains(&keys[index]), "a group overlapped");
                    running.push(keys[index]);
                }
                if keys[index] == "slow" {
                    thread::sleep(Duration::from_millis(30));
                }
                running.lock().unwrap().retain(|key| *key != keys[index]);
                Ok(index * 10)
            },
            |so_far| seen.lock().unwrap().push(so_far.iter().flatten().count()),
        )
        .unwrap();
        assert_eq!(results, [0, 10, 20, 30, 40]);
        assert_eq!(*seen.lock().unwrap(), [1, 2, 3, 4, 5]);
    }

    #[test]
    fn an_error_stops_new_recipes_and_reports_the_earliest() {
        let ran = AtomicUsize::new(0);
        let error = run(
            &groups(&[0, 1, 2, 3]),
            1,
            |index| {
                ran.fetch_add(1, Ordering::SeqCst);
                if index == 1 {
                    Err("broken".into())
                } else {
                    Ok(())
                }
            },
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error, "broken");
        assert_eq!(ran.load(Ordering::SeqCst), 2);
    }
}
