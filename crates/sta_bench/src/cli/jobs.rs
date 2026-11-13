//! `--jobs`: run cases on a small worker pool (docs/TESTBENCH.md, "Two tiers").
//!
//! Tier-1 cases are in-process and light, so `--jobs N` threads run them.
//! Tier-2 cases start a GPU `sta64` process; at most `--gpu-jobs` of those
//! run at once (default 1, whatever `--jobs` says), and each child runs at low
//! priority so a bench never starves the machine it runs on. Results come
//! back in the order the cases were given, so a run is the same at any `--jobs`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::{Opts, failed_case, tier2};
use crate::run::{self, CaseResult, RunConfig, load_scenario, tier};

/// Default worker count: tier 1 is light, but the user's machine is shared.
pub(crate) const DEFAULT_JOBS: usize = 2;

pub(crate) fn jobs(o: &Opts) -> usize {
    o.jobs.unwrap_or(DEFAULT_JOBS).max(1)
}

pub(crate) fn gpu_jobs(o: &Opts) -> usize {
    o.gpu_jobs.unwrap_or(1).max(1)
}

/// A counting semaphore (std has none).
struct Gate {
    free: Mutex<usize>,
    cv: Condvar,
}

impl Gate {
    fn new(n: usize) -> Gate {
        Gate { free: Mutex::new(n), cv: Condvar::new() }
    }
    fn acquire(&self) {
        let mut f = self.free.lock().unwrap();
        while *f == 0 {
            f = self.cv.wait(f).unwrap();
        }
        *f -= 1;
    }
    fn release(&self) {
        *self.free.lock().unwrap() += 1;
        self.cv.notify_one();
    }
}

/// Releases the gate on drop, so a panic in a case cannot hang the run.
struct Slot<'a>(&'a Gate);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// What to run for one scenario file.
pub(crate) struct Job {
    pub path: PathBuf,
    pub name: String,
}

/// For each job, the jobs that must finish first: those that save the file
/// its scene starts from (`Start=snapshot:F`, `Start=replay:F`). A job never
/// waits on itself; a file nothing in `list` saves adds no wait (the job
/// reports the missing file).
fn dependencies(cfg: &RunConfig, list: &[Job]) -> Vec<Vec<usize>> {
    let files: Vec<_> = list
        .iter()
        .map(|j| load_scenario(cfg, &j.path).map(|sc| run::data_files(cfg, &sc)).unwrap_or_default())
        .collect();
    files
        .iter()
        .enumerate()
        .map(|(i, (_, reads))| match reads {
            Some(f) => (0..list.len()).filter(|&k| k != i && files[k].0.contains(f)).collect(),
            None => Vec::new(),
        })
        .collect()
}

/// Which job a free worker takes: the first unstarted one whose
/// dependencies are done (so input order is kept when nothing depends on
/// anything). `None` while some are unstarted but all wait on running jobs.
/// With nothing running (a dependency cycle) the first unstarted one goes,
/// and fails on its missing file rather than hanging the run.
fn pick(needs: &[Vec<usize>], started: &[bool], done: &[bool]) -> Option<Option<usize>> {
    if started.iter().all(|&s| s) {
        return Some(None);
    }
    let free = (0..needs.len()).find(|&i| !started[i] && needs[i].iter().all(|&d| done[d]));
    if free.is_some() {
        return Some(free);
    }
    let running = started.iter().zip(done).any(|(&s, &d)| s && !d);
    if running { None } else { Some((0..needs.len()).find(|&i| !started[i])) }
}

struct Queue {
    started: Vec<bool>,
    done: Vec<bool>,
}

/// Run `jobs` on the worker pool, a job after the ones it depends on
/// ([`dependencies`]); tier-2 work only when `allow_tier2`.
/// Returns each case with its wall time, in input order.
pub(crate) fn run_all(
    cfg: &Arc<RunConfig>,
    o: &Opts,
    list: &[Job],
    staged: &Path,
    allow_tier2: bool,
) -> Vec<(CaseResult, Duration)> {
    let slots: Vec<Mutex<Option<(CaseResult, Duration)>>> = list.iter().map(|_| Mutex::new(None)).collect();
    let needs = dependencies(cfg, list);
    let queue = Mutex::new(Queue { started: vec![false; list.len()], done: vec![false; list.len()] });
    let changed = Condvar::new();
    let gate = Gate::new(gpu_jobs(o));
    let workers = jobs(o).min(list.len()).max(1);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = {
                        let mut q = queue.lock().unwrap();
                        loop {
                            match pick(&needs, &q.started, &q.done) {
                                Some(Some(i)) => {
                                    q.started[i] = true;
                                    break Some(i);
                                }
                                Some(None) => break None,
                                None => q = changed.wait(q).unwrap(),
                            }
                        }
                    };
                    let Some(i) = i else { break };
                    let start = Instant::now();
                    // A panicking case must still release its dependents.
                    let _done = Finished { queue: &queue, changed: &changed, i };
                    let case = one(cfg, o, &list[i], staged, allow_tier2, &gate);
                    *slots[i].lock().unwrap() = Some((case, start.elapsed()));
                }
            });
        }
    });
    slots.into_iter().map(|m| m.into_inner().unwrap().expect("every job ran")).collect()
}

/// Marks a job done on drop and wakes the workers waiting on it.
struct Finished<'a> {
    queue: &'a Mutex<Queue>,
    changed: &'a Condvar,
    i: usize,
}

impl Drop for Finished<'_> {
    fn drop(&mut self) {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).done[self.i] = true;
        self.changed.notify_all();
    }
}

fn one(cfg: &Arc<RunConfig>, o: &Opts, job: &Job, staged: &Path, allow_tier2: bool, gate: &Gate) -> CaseResult {
    let sc = match load_scenario(cfg, &job.path) {
        Ok(sc) => sc,
        Err(e) => return failed_case(&job.name, e),
    };
    let mut case = run::run_scenario(cfg, &sc, &job.name);
    if allow_tier2 && (o.tier == Some(2) || (o.tier.is_none() && tier(&sc) == Some(2))) {
        gate.acquire();
        let _slot = Slot(gate);
        tier2::run_case(cfg, &sc, &mut case, o, staged);
    }
    case
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_reader_waits_for_its_saver_whatever_the_order() {
        // Job 0 reads what job 2 saves; job 1 is independent.
        let needs = vec![vec![2], vec![], vec![]];
        let (mut started, mut done) = (vec![false; 3], vec![false; 3]);
        assert_eq!(pick(&needs, &started, &done), Some(Some(1)), "the reader is skipped");
        started[1] = true;
        assert_eq!(pick(&needs, &started, &done), Some(Some(2)));
        started[2] = true;
        assert_eq!(pick(&needs, &started, &done), None, "the reader waits while the saver runs");
        done[2] = true;
        assert_eq!(pick(&needs, &started, &done), Some(Some(0)));
        started[0] = true;
        assert_eq!(pick(&needs, &started, &done), Some(None), "everything has started");
    }

    #[test]
    fn a_dependency_cycle_runs_instead_of_hanging() {
        let needs = vec![vec![1], vec![0]];
        assert_eq!(pick(&needs, &[false; 2], &[false; 2]), Some(Some(0)));
    }

    #[test]
    fn gate_never_lets_more_than_its_slots_in() {
        let gate = Gate::new(1);
        let (now, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        std::thread::scope(|s| {
            for _ in 0..6 {
                s.spawn(|| {
                    for _ in 0..20 {
                        gate.acquire();
                        let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(n, Ordering::SeqCst);
                        std::thread::yield_now();
                        now.fetch_sub(1, Ordering::SeqCst);
                        gate.release();
                    }
                });
            }
        });
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn defaults_are_conservative() {
        let o = Opts::default();
        assert_eq!((jobs(&o), gpu_jobs(&o)), (2, 1));
        let o = Opts { jobs: Some(0), gpu_jobs: Some(0), ..Opts::default() };
        assert_eq!((jobs(&o), gpu_jobs(&o)), (1, 1));
    }
}
