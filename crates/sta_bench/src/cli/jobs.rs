//! `--jobs`: run cases on a small worker pool (docs/TESTBENCH.md, "Two tiers").
//!
//! Tier-1 cases are in-process and light, so `--jobs N` threads run them.
//! Tier-2 cases start a GPU `sta64` process; at most `--gpu-jobs` of those
//! run at once (default 1, whatever `--jobs` says), and each child runs at low
//! priority so a bench never starves the machine it runs on. Results come
//! back in the order the cases were given, so a run is the same at any `--jobs`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// What to run for one scenario file.
pub(crate) struct Job {
    pub path: PathBuf,
    pub name: String,
}

/// Run `jobs` in the order given; tier-2 work only when `allow_tier2`.
/// Returns each case with its wall time, in input order.
pub(crate) fn run_all(
    cfg: &Arc<RunConfig>,
    o: &Opts,
    list: &[Job],
    staged: &Path,
    allow_tier2: bool,
) -> Vec<(CaseResult, Duration)> {
    let slots: Vec<Mutex<Option<(CaseResult, Duration)>>> = list.iter().map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let gate = Gate::new(gpu_jobs(o));
    let workers = jobs(o).min(list.len()).max(1);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(job) = list.get(i) else { break };
                    let start = Instant::now();
                    let case = one(cfg, o, job, staged, allow_tier2, &gate);
                    *slots[i].lock().unwrap() = Some((case, start.elapsed()));
                }
            });
        }
    });
    slots.into_iter().map(|m| m.into_inner().unwrap().expect("every job ran")).collect()
}

fn one(cfg: &Arc<RunConfig>, o: &Opts, job: &Job, staged: &Path, allow_tier2: bool, gate: &Gate) -> CaseResult {
    let sc = match load_scenario(cfg, &job.path) {
        Ok(sc) => sc,
        Err(e) => return failed_case(&job.name, e),
    };
    let mut case = run::run_scenario(cfg, &sc, &job.name);
    if allow_tier2 && (o.tier == Some(2) || (o.tier.is_none() && tier(&sc) == Some(2))) {
        gate.acquire();
        tier2::run_case(cfg, &sc, &mut case, o, staged);
        gate.release();
    }
    case
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

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
