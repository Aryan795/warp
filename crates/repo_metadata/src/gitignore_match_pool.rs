//! A small, fixed set of OS threads dedicated to evaluating gitignore matches.
//!
//! `regex_automata::util::pool::Pool` (the scratch-cache pool backing every compiled
//! `Gitignore` matcher; see `gitignore_cache`) shards its retained caches by
//! `thread_id % 8` and never shrinks. `get_slow` pops only from the caller's own shard,
//! so a caller landing on a shard another caller already filled mints and keeps a fresh
//! cache regardless of how many callers are concurrently in flight — retention is keyed
//! by the *set of distinct OS threads* that have ever called into a matcher's pool, not
//! by how many call it at once. Limiting concurrency (e.g. a semaphore) does not bound
//! this: an unlucky sequence of *sequential* callers on distinct threads can still fill
//! every shard.
//!
//! This module bounds the thread set directly instead: every gitignore match, from any
//! caller, runs on one of [`THREAD_COUNT`] dedicated worker threads that are spawned
//! once and live for the process's lifetime. Since `regex_automata` assigns each OS
//! thread a stable ID on its first call into any pool and never changes it, restricting
//! the caller set to `THREAD_COUNT` threads bounds every matcher's retained cache count
//! to `THREAD_COUNT + 1` (the dedicated threads, plus the one "owner" slot claimed by
//! whichever thread calls the matcher first) — a constant, independent of the number of
//! matchers, callers, or background/runtime worker threads in the process.

use std::collections::VecDeque;

use parking_lot::{Condvar, Mutex};

/// Number of dedicated OS threads ever allowed to touch a `Gitignore`'s `regex_automata`
/// pool. This bounds every matcher's worst-case retained cache count to
/// `THREAD_COUNT + 1` regardless of caller concurrency (see module docs): with globset's
/// `hybrid_cache_capacity` of 10 MiB per compiled matcher, that is at most
/// `(THREAD_COUNT + 1) * 10 MiB` = 90 MiB retained per matcher in the pathological case
/// (most real `.gitignore` patterns are literal/prefix/suffix and never reach
/// `regex_automata` at all, so this ceiling is rarely approached in practice). Matching
/// is a fast, CPU-only regex check relative to the directory I/O surrounding it in every
/// caller, so 8 dedicated threads — matching `regex_automata`'s own `MAX_POOL_STACKS`
/// shard count — keeps real work overlapping during watcher storms and embedding scans
/// without letting the thread set, and therefore the worst-case retained memory per
/// matcher, grow with the number of background executor or rayon workers.
const THREAD_COUNT: usize = 8;

type Job = Box<dyn FnOnce() + Send>;

struct WorkQueue {
    jobs: Mutex<VecDeque<Job>>,
    ready: Condvar,
}

fn worker_loop(queue: &WorkQueue) {
    loop {
        let job = {
            let mut jobs = queue.jobs.lock();
            loop {
                if let Some(job) = jobs.pop_front() {
                    break job;
                }
                queue.ready.wait(&mut jobs);
            }
        };
        job();
    }
}

fn build_queue() -> &'static WorkQueue {
    // Leaked deliberately: the dedicated threads spawned below borrow it for the life of
    // the process, which is exactly the lifetime this pool is meant to have (see module
    // docs — the thread set must stay fixed, so these threads are never torn down).
    let queue: &'static WorkQueue = Box::leak(Box::new(WorkQueue {
        jobs: Mutex::new(VecDeque::new()),
        ready: Condvar::new(),
    }));
    for index in 0..THREAD_COUNT {
        std::thread::Builder::new()
            .name(format!("gitignore-match-{index}"))
            .spawn(move || worker_loop(queue))
            .expect("failed to spawn dedicated gitignore-match thread");
    }
    queue
}

/// Runs `f` on one of the dedicated gitignore-match threads and blocks the calling
/// thread until it completes.
///
/// Safe to call from any context — an async task, a rayon worker, or a plain sync
/// callback — since the dedicated threads are independent, plain OS threads that never
/// depend on the caller's own executor to make progress. There is no re-entrancy or
/// deadlock risk as long as `f` never calls back into this function; matching is a leaf
/// computation and never does.
pub(crate) fn run<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    static QUEUE: std::sync::LazyLock<&'static WorkQueue> = std::sync::LazyLock::new(build_queue);

    let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
    {
        let mut jobs = QUEUE.jobs.lock();
        jobs.push_back(Box::new(move || {
            let _ = result_tx.send(f());
        }));
    }
    QUEUE.ready.notify_one();
    result_rx
        .recv()
        .expect("a dedicated gitignore-match thread panicked without sending a result")
}

#[cfg(test)]
#[path = "gitignore_match_pool_tests.rs"]
mod tests;
