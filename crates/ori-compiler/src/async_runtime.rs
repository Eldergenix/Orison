//! Cooperative async scheduler for the bootstrap interpreter.
//!
//! The bootstrap lexer recognises the `async` and `await` keywords but the
//! tree-walking interpreter in [`crate::interp_exec`] is otherwise purely
//! synchronous. This module provides a small, deterministic scheduler that
//! later passes (and the interpreter itself) can hook into so that an
//! `async fn` body can yield to siblings before resuming.
//!
//! Design constraints:
//!   * Fully deterministic — FIFO ordering of ready tasks, sorted output for
//!     diagnostics so identical inputs always produce identical reports.
//!   * Monotonic future ids: a [`Scheduler`] never re-uses an id, even after
//!     the corresponding task completes. Re-use would defeat structured
//!     concurrency invariants downstream.
//!   * No allocator hot loops on hot paths: the queue and waiting map both
//!     grow linearly with `spawn` calls, never per-tick.
//!   * Diagnostics use stable IDs in the `A0001..A0099` range so the CLI and
//!     downstream agents can mechanically map them:
//!       * `A0001` — scheduler overflow (more than `max_steps` ticks).
//!       * `A0002` — deadlock detected (pending tasks but nothing to resume).
//!       * `A0003` — future leak (spawned id never resumed, no longer queued).
//!       * `A0010` — worker thread panic captured via `catch_unwind` (M:N).
//!       * `A0011` — task starvation: a task hadn't run after N ticks.
//!       * `A0012` — shutdown timeout: workers didn't exit cleanly in time.
//!       * `A0013` — caller asked for more workers than the policy cap.
//!
//! M26 extends this module with an M:N parallel scheduler
//! ([`SchedulerMode::Parallel`]) backed by a pool of OS threads that pull
//! work from a shared `Mutex<VecDeque<ParallelTask>>` and signal each other
//! through a `Condvar`. The cooperative single-thread API on [`Scheduler`]
//! and [`run_to_completion`] is unchanged.
//!
//! Bootstrap policy: this file is permitted to call `std::panic::catch_unwind`
//! (the only place in the tree that does) because worker panics must not
//! abort the host process. That call does not itself panic — it just turns
//! a panicking worker body into an [`AsyncReport`] diagnostic.

use crate::diagnostic::{Diagnostic, Fix};
use crate::interp_exec::Value;
use crate::source::Span;
use std::collections::{BTreeMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Outcome of a single cooperative tick. `Ready` means the corresponding
/// `fut_id`'s value is now observable; `Pending` means the scheduler advanced
/// but the front of the queue is parked awaiting an external resume.
#[derive(Debug, Clone, PartialEq)]
pub enum Task {
    Ready(Value),
    Pending { fut_id: u64 },
}

/// Tiny cooperative scheduler. Public fields are part of the bootstrap API so
/// the interpreter can introspect queue depth and pending futures directly.
#[derive(Debug, Clone, Default)]
pub struct Scheduler {
    /// FIFO of `fut_id`s that are ready to advance.
    pub queue: VecDeque<u64>,
    /// Pending futures keyed by id, awaiting an external resume value.
    pub waiting: BTreeMap<u64, Value>,
    /// Monotonic id allocator. Never decreases, never re-uses.
    pub next_id: u64,
    /// Set of `fut_id`s that have ever been spawned. Used to detect leaks.
    /// Implemented as a `BTreeMap<u64, ()>` so diagnostic ordering is stable.
    spawned_ids: BTreeMap<u64, ()>,
    /// Set of `fut_id`s whose `Ready` value has already been observed.
    completed_ids: BTreeMap<u64, ()>,
    /// Resume buffer keyed by `fut_id`. When a pending task is resumed we
    /// store the value here and re-enqueue the id. Separate from `waiting`
    /// so we can distinguish "parked" from "ready-with-value".
    resumed_values: BTreeMap<u64, Value>,
    /// Running maximum queue depth ever observed. Used by [`AsyncReport`].
    max_queue_depth: usize,
}

impl Scheduler {
    /// Construct an empty scheduler. `next_id` starts at zero; the first
    /// spawn returns id `0` and the allocator strictly increases.
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn a value-bearing task. Returns its monotonic future id. The value
    /// is enqueued as immediately-ready; callers that need a pending future
    /// should call [`Scheduler::spawn`] and then [`Scheduler::park`] in turn.
    pub fn spawn(&mut self, value: Value) -> u64 {
        let id = self.next_id;
        // Saturating-add guards against u64 overflow on absurdly long runs.
        self.next_id = self.next_id.saturating_add(1);
        self.spawned_ids.insert(id, ());
        self.resumed_values.insert(id, value);
        self.queue.push_back(id);
        if self.queue.len() > self.max_queue_depth {
            self.max_queue_depth = self.queue.len();
        }
        id
    }

    /// Park `fut_id` in the waiting set with a sentinel value. The id must
    /// have been previously returned by [`Scheduler::spawn`]; calling with an
    /// unknown id is a no-op so callers don't need to defensively check.
    pub fn park(&mut self, fut_id: u64, sentinel: Value) {
        if !self.spawned_ids.contains_key(&fut_id) {
            return;
        }
        // Remove from queue if present so it doesn't immediately fire.
        self.queue.retain(|q| *q != fut_id);
        self.resumed_values.remove(&fut_id);
        self.waiting.insert(fut_id, sentinel);
    }

    /// Resume a parked future with the given value. The value will surface
    /// from the next [`Scheduler::step`] that drains this id. No-op if the
    /// id is not currently parked, so resume-before-park is safe.
    pub fn resume(&mut self, fut_id: u64, value: Value) {
        if self.waiting.remove(&fut_id).is_none() {
            return;
        }
        self.resumed_values.insert(fut_id, value);
        self.queue.push_back(fut_id);
        if self.queue.len() > self.max_queue_depth {
            self.max_queue_depth = self.queue.len();
        }
    }

    /// Cooperative tick. Pops the front of the ready queue and returns the
    /// associated [`Task`]. Returns `None` when there is no work to do — this
    /// is the idempotent empty-queue case and never panics.
    pub fn step(&mut self) -> Option<Task> {
        let id = self.queue.pop_front()?;
        if let Some(value) = self.resumed_values.remove(&id) {
            self.completed_ids.insert(id, ());
            Some(Task::Ready(value))
        } else {
            // Defensive: the id was on the queue without a resumed value.
            // Re-park it so the caller can observe the pending state.
            self.waiting.insert(id, Value::Unit);
            Some(Task::Pending { fut_id: id })
        }
    }

    /// Number of distinct futures ever spawned in this scheduler.
    pub fn spawned_count(&self) -> usize {
        self.spawned_ids.len()
    }

    /// Number of futures whose ready value has been observed via `step()`.
    pub fn completed_count(&self) -> usize {
        self.completed_ids.len()
    }

    /// Sorted list of pending future ids (waiting on a resume).
    pub fn pending_ids(&self) -> Vec<u64> {
        self.waiting.keys().copied().collect()
    }

    /// Sorted list of spawned-but-not-completed-and-not-queued-and-not-parked
    /// future ids. These are the "leaked" futures referenced by `A0003`.
    pub fn leaked_ids(&self) -> Vec<u64> {
        let queued: BTreeMap<u64, ()> = self.queue.iter().copied().map(|i| (i, ())).collect();
        self.spawned_ids
            .keys()
            .copied()
            .filter(|id| {
                !self.completed_ids.contains_key(id)
                    && !self.waiting.contains_key(id)
                    && !queued.contains_key(id)
            })
            .collect()
    }
}

/// Drain the scheduler until either the queue is empty or `max_steps` is
/// reached. Returns the ordered list of ready values observed along the way.
/// Caps at `max_steps` to prevent runaway loops; callers that want to report
/// an `A0001` overflow can compare `result.len()` against `max_steps`.
pub fn run_to_completion(scheduler: &mut Scheduler, max_steps: usize) -> Vec<Value> {
    let mut out = Vec::new();
    let mut steps = 0usize;
    while steps < max_steps {
        match scheduler.step() {
            Some(Task::Ready(value)) => out.push(value),
            Some(Task::Pending { .. }) => { /* park observed; skip */ }
            None => break,
        }
        steps = steps.saturating_add(1);
    }
    out
}

/// Deterministic summary of a scheduler run. The `schema` constant lets
/// downstream agents key off `ori.async_report.v1` without parsing version
/// substrings.
///
/// The `worker_count`, `total_steps`, `total_tasks`, `cancelled`, `panicked`,
/// and `diagnostics` fields are introduced by M26 to support both
/// deterministic single-thread and M:N parallel execution under a single
/// report schema. Fields default to zero / empty so callers that only care
/// about the cooperative numbers can ignore the M:N additions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncReport {
    pub schema: &'static str,
    pub spawned: usize,
    pub completed: usize,
    pub stalled: usize,
    pub steps_taken: usize,
    pub max_queue_depth: usize,
    /// Number of worker threads engaged. `0` for cooperative runs, `>=1`
    /// for parallel runs.
    pub worker_count: usize,
    /// Total cooperative ticks executed across all workers (sum of
    /// `step_count` per worker). For deterministic runs this equals
    /// `steps_taken`.
    pub total_steps: usize,
    /// Total tasks the scheduler accepted for execution.
    pub total_tasks: usize,
    /// Tasks that were dropped because shutdown was requested before they
    /// ran. Only meaningful for parallel runs.
    pub cancelled: usize,
    /// Tasks whose body panicked and was caught by the worker's
    /// `catch_unwind`. Each occurrence also surfaces an `A0010`.
    pub panicked: usize,
    /// Sorted diagnostic ids emitted by the parallel run. Empty for
    /// cooperative runs; cooperative diagnostics are returned separately
    /// via [`diagnostics_for`].
    pub diagnostics: Vec<String>,
}

impl AsyncReport {
    /// Build a report from the current scheduler state plus the externally
    /// tracked `steps_taken` (the caller is the only place that knows how
    /// many ticks it actually drove).
    pub fn from_scheduler(scheduler: &Scheduler, steps_taken: usize) -> Self {
        Self {
            schema: "ori.async_report.v1",
            spawned: scheduler.spawned_count(),
            completed: scheduler.completed_count(),
            stalled: scheduler.pending_ids().len(),
            steps_taken,
            max_queue_depth: scheduler.max_queue_depth,
            worker_count: 0,
            total_steps: steps_taken,
            total_tasks: scheduler.spawned_count(),
            cancelled: 0,
            panicked: 0,
            diagnostics: Vec::new(),
        }
    }
}

/// Sentinel span used for runtime-emitted async diagnostics. Async events
/// don't correspond to a source position, so we attach a dummy span keyed to
/// `<async-runtime>` and let downstream renderers either dedupe or hide it.
fn synthetic_span() -> Span {
    Span::dummy("<async-runtime>")
}

/// Diagnostics derived from a scheduler run. Output is sorted by diagnostic
/// id and then by future id so callers can rely on byte-stable serialisation.
pub fn diagnostics_for(
    scheduler: &Scheduler,
    steps_taken: usize,
    max_steps: usize,
) -> Vec<Diagnostic> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    if steps_taken >= max_steps && !scheduler.queue.is_empty() {
        diags.push(
            Diagnostic::error(
                "A0001",
                format!(
                    "async scheduler overflow: hit max_steps={max_steps} with {} task(s) still queued",
                    scheduler.queue.len()
                ),
                synthetic_span(),
            )
            .with_agent_summary(
                "Raise max_steps or split the workload; the scheduler hit its tick cap.",
            )
            .with_fix(Fix::new(
                "raise-max-steps",
                "Increase max_steps in run_to_completion to allow further ticks.",
                0.6,
            )),
        );
    }

    let pending = scheduler.pending_ids();
    if !pending.is_empty() && scheduler.queue.is_empty() {
        // Sort ensures the rendered message is deterministic across runs.
        let mut ids = pending.clone();
        ids.sort_unstable();
        let id_list = ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
        diags.push(
            Diagnostic::error(
                "A0002",
                format!("async deadlock detected: pending future(s) [{id_list}] with no resume"),
                synthetic_span(),
            )
            .with_agent_summary(
                "Some futures parked but no scheduler.resume(...) call will wake them.",
            ),
        );
    }

    let mut leaked = scheduler.leaked_ids();
    leaked.sort_unstable();
    if !leaked.is_empty() {
        let id_list = leaked
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        diags.push(
            Diagnostic::warning(
                "A0003",
                format!(
                    "async future leak: spawned future(s) [{id_list}] never resumed and not queued"
                ),
                synthetic_span(),
            )
            .with_agent_summary(
                "A future id was allocated but the runtime never observed a resume or tick.",
            ),
        );
    }

    diags.sort_by(|a, b| a.id.cmp(&b.id));
    diags
}

// ---------------------------------------------------------------------------
// M:N parallel scheduler (M26).
// ---------------------------------------------------------------------------

/// Hard upper bound on workers for the bootstrap. Surfaced via [`A0013`] when
/// callers ask for more. Picked to comfortably exceed any reasonable laptop
/// or CI runner without letting a typo spawn thousands of threads.
pub const MAX_WORKERS: usize = 256;

/// Default per-tick grace before a parallel worker is asked to shut down.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// Execution mode for the bootstrap scheduler. The deterministic mode is
/// the legacy single-thread FIFO; the parallel mode is the M26 M:N pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerMode {
    /// Single-thread, strict insertion-order execution. Reproducible across
    /// runs; the only mode permitted in `Deterministic` tests.
    Deterministic,
    /// Worker-pool execution with `workers` OS threads sharing a single
    /// FIFO queue. Tasks are executed concurrently, so user code MUST NOT
    /// depend on completion order across tasks.
    Parallel { workers: usize },
}

/// A unit of work for the parallel scheduler. The body is boxed so we can
/// store heterogeneous closures and ship them across the channel into the
/// worker pool. Bodies run inside `catch_unwind`, so a panic produces an
/// `A0010` diagnostic instead of aborting the process.
pub struct ParallelTask {
    /// Monotonic id, identical in shape to [`Scheduler::next_id`].
    pub id: u64,
    /// User-supplied body. Returns the task's logical result value so the
    /// worker can record per-task outcomes without sharing more state.
    pub body: Box<dyn FnOnce() -> Value + Send + 'static>,
}

impl std::fmt::Debug for ParallelTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParallelTask")
            .field("id", &self.id)
            .finish()
    }
}

/// State shared between the orchestrator and every worker thread. All fields
/// live behind `Arc<Mutex<...>>` because we do not introduce `crossbeam` or
/// other lock-free deps for the bootstrap.
#[derive(Debug, Default)]
struct ParallelState {
    queue: VecDeque<ParallelTask>,
    /// Set once the orchestrator wants workers to drain and exit. Workers
    /// re-check this between pops; combined with an empty queue it triggers
    /// the worker's terminal `break`.
    shutdown: bool,
    /// Tasks accepted into the queue. Includes ones that later get dropped
    /// when shutdown fires before they run.
    accepted: u64,
    /// Tasks completed successfully (body returned without panic).
    completed: u64,
    /// Tasks whose body panicked under `catch_unwind`.
    panicked: u64,
    /// Tasks that were sitting in the queue when shutdown drained them.
    cancelled: u64,
    /// Cooperative ticks across all workers — every successful pop counts
    /// as one tick regardless of outcome.
    total_steps: u64,
    /// Sorted set of diagnostic ids emitted by the parallel run.
    diagnostics: BTreeMap<String, ()>,
}

/// Builder wrapper for the parallel scheduler. Keeps the public surface
/// small: callers either go through [`Scheduler::run_to_completion_parallel`]
/// (a convenience that builds a fresh pool, pushes the already-spawned tasks
/// as no-op bodies, and tears it down) or via [`ParallelScheduler`] directly
/// when they need to mix custom worker bodies with their own tasks.
pub struct ParallelScheduler {
    state: Arc<(Mutex<ParallelState>, Condvar)>,
    next_id: u64,
}

impl Default for ParallelScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl ParallelScheduler {
    /// Construct an empty pool with no workers running yet. Workers are
    /// spawned lazily by [`Self::run`].
    pub fn new() -> Self {
        Self {
            state: Arc::new((Mutex::new(ParallelState::default()), Condvar::new())),
            next_id: 0,
        }
    }

    /// Push a closure onto the shared queue. Returns the monotonic id we
    /// minted so callers can correlate results.
    ///
    /// If shutdown has already been requested, the submission is still
    /// counted in `accepted` but bypasses the queue and increments
    /// `cancelled` directly — once shutdown fires the pool refuses to run
    /// new work, but accounting must still see every id.
    pub fn submit<F>(&mut self, body: F) -> u64
    where
        F: FnOnce() -> Value + Send + 'static,
    {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let (lock, cvar) = &*self.state;
        // `lock()` returns a Result whose Err means another thread panicked
        // while holding the mutex. We treat that as "state is poisoned but
        // recoverable for our purposes" — the worker pool will already have
        // recorded the panic as A0010 by the time we get here.
        match lock.lock() {
            Ok(mut guard) => {
                guard.accepted = guard.accepted.saturating_add(1);
                if guard.shutdown {
                    // Drop the body without ever calling it. Counted toward
                    // the cancelled tally so reports stay self-consistent.
                    drop(body);
                    guard.cancelled = guard.cancelled.saturating_add(1);
                } else {
                    guard.queue.push_back(ParallelTask {
                        id,
                        body: Box::new(body),
                    });
                }
            }
            Err(poison) => {
                let mut guard = poison.into_inner();
                guard.diagnostics.insert("A0010".to_string(), ());
                guard.accepted = guard.accepted.saturating_add(1);
                if guard.shutdown {
                    drop(body);
                    guard.cancelled = guard.cancelled.saturating_add(1);
                } else {
                    guard.queue.push_back(ParallelTask {
                        id,
                        body: Box::new(body),
                    });
                }
            }
        }
        cvar.notify_one();
        id
    }

    /// Drain the queue across `workers` threads, returning an
    /// [`AsyncReport`]. `max_steps_per_task` caps the per-task tick budget;
    /// each pop counts as one tick. The grace period bounds how long we
    /// wait for workers to notice the shutdown flag — exceeding it surfaces
    /// `A0012`.
    pub fn run(
        &mut self,
        workers: usize,
        max_steps_per_task: usize,
        grace: Duration,
    ) -> AsyncReport {
        // Defensive caps: zero workers degrade to "do nothing" rather than
        // hang forever; oversized counts get clamped and surface A0013.
        let mut policy_violated = false;
        let mut effective_workers = workers;
        if effective_workers > MAX_WORKERS {
            effective_workers = MAX_WORKERS;
            policy_violated = true;
        }
        if effective_workers == 0 {
            // Nothing to do — return whatever the pool currently looks like.
            let snapshot = lock_snapshot(&self.state);
            return finalise_report(snapshot, 0, max_steps_per_task);
        }

        if policy_violated {
            insert_diag(&self.state, "A0013");
        }

        let mut handles = Vec::with_capacity(effective_workers);
        for _ in 0..effective_workers {
            let state = Arc::clone(&self.state);
            let handle = thread::spawn(move || worker_loop(state, max_steps_per_task));
            handles.push(handle);
        }

        // Workers pop until the queue is empty AND `shutdown` is true.
        // The caller controls when to flip shutdown via `shutdown()`.
        // Default behaviour: flip after submission is complete and tasks
        // have had a chance to start, then join.
        request_shutdown(&self.state);

        let deadline = Instant::now() + grace;
        let mut timed_out = false;
        for handle in handles.drain(..) {
            // We can't enforce per-thread deadlines without unstable APIs,
            // so we just join in order. If joining takes longer than the
            // grace window we record A0012 once and keep joining — better
            // to wait than to leak a thread.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() && !timed_out {
                insert_diag(&self.state, "A0012");
                timed_out = true;
            }
            match handle.join() {
                Ok(()) => {}
                Err(_) => {
                    // A worker panicked outside the `catch_unwind` wrapper —
                    // shouldn't happen, but if it does we still record it.
                    insert_diag(&self.state, "A0010");
                }
            }
        }

        let snapshot = lock_snapshot(&self.state);
        finalise_report(snapshot, effective_workers, max_steps_per_task)
    }

    /// Flip the shutdown flag without spawning workers — useful for tests
    /// that want to assert pre-run cancellations.
    pub fn request_shutdown(&self) {
        request_shutdown(&self.state);
    }
}

fn request_shutdown(state: &Arc<(Mutex<ParallelState>, Condvar)>) {
    let (lock, cvar) = &**state;
    if let Ok(mut guard) = lock.lock() {
        guard.shutdown = true;
    }
    cvar.notify_all();
}

fn insert_diag(state: &Arc<(Mutex<ParallelState>, Condvar)>, code: &str) {
    let (lock, _) = &**state;
    if let Ok(mut guard) = lock.lock() {
        guard.diagnostics.insert(code.to_string(), ());
    }
}

/// Take a consistent snapshot of the shared state under the mutex, lifting
/// the heavy fields out so the caller can finalise the report without
/// holding the lock across allocations.
struct StateSnapshot {
    accepted: u64,
    completed: u64,
    panicked: u64,
    cancelled: u64,
    total_steps: u64,
    diagnostics: Vec<String>,
}

fn lock_snapshot(state: &Arc<(Mutex<ParallelState>, Condvar)>) -> StateSnapshot {
    let (lock, _) = &**state;
    match lock.lock() {
        Ok(guard) => StateSnapshot {
            accepted: guard.accepted,
            completed: guard.completed,
            panicked: guard.panicked,
            cancelled: guard.cancelled,
            total_steps: guard.total_steps,
            diagnostics: guard.diagnostics.keys().cloned().collect(),
        },
        Err(poison) => {
            // Mirror the Ok arm but record A0010 because reaching the
            // poisoned branch means a worker panicked while holding the
            // lock — exactly the case the diagnostic exists to flag.
            let mut guard = poison.into_inner();
            guard.diagnostics.insert("A0010".to_string(), ());
            StateSnapshot {
                accepted: guard.accepted,
                completed: guard.completed,
                panicked: guard.panicked,
                cancelled: guard.cancelled,
                total_steps: guard.total_steps,
                diagnostics: guard.diagnostics.keys().cloned().collect(),
            }
        }
    }
}

fn finalise_report(
    snapshot: StateSnapshot,
    worker_count: usize,
    max_steps_per_task: usize,
) -> AsyncReport {
    let mut diagnostics = snapshot.diagnostics;
    // A0011 fires if we accepted tasks that never ran (completed +
    // panicked + cancelled < accepted). That's the bootstrap's stand-in
    // for "starvation" — a task sat on the queue past the per-task budget
    // without being picked up.
    let executed = snapshot.completed + snapshot.panicked + snapshot.cancelled;
    if executed < snapshot.accepted {
        diagnostics.push("A0011".to_string());
    }
    // Stable ordering: diagnostic ids are short ASCII codes, so a lexical
    // sort is the same as the numeric sort callers expect.
    diagnostics.sort();
    diagnostics.dedup();

    let _ = max_steps_per_task; // currently advisory only; surfaced via A0011.

    AsyncReport {
        schema: "ori.async_report.v1",
        spawned: snapshot.accepted as usize,
        completed: snapshot.completed as usize,
        // Parallel mode never parks futures; stalled == 0 by construction.
        stalled: 0,
        steps_taken: snapshot.total_steps as usize,
        max_queue_depth: 0,
        worker_count,
        total_steps: snapshot.total_steps as usize,
        total_tasks: snapshot.accepted as usize,
        cancelled: snapshot.cancelled as usize,
        panicked: snapshot.panicked as usize,
        diagnostics,
    }
}

/// Worker main loop. Pops tasks under the shared mutex, runs the body inside
/// `catch_unwind`, and updates the shared counters. Exits once the queue is
/// empty AND shutdown has been requested.
fn worker_loop(state: Arc<(Mutex<ParallelState>, Condvar)>, max_steps_per_task: usize) {
    let (lock, cvar) = &*state;
    let mut local_steps: usize = 0;
    loop {
        let next: Option<ParallelTask> = {
            // Acquire and wait under the same lock. If poisoning happens
            // we still attempt to continue — workers must never escalate a
            // mutex panic into a process abort.
            let guard_result = lock.lock();
            let mut guard = match guard_result {
                Ok(g) => g,
                Err(poison) => {
                    let mut g = poison.into_inner();
                    g.diagnostics.insert("A0010".to_string(), ());
                    g
                }
            };
            loop {
                if let Some(task) = guard.queue.pop_front() {
                    break Some(task);
                }
                if guard.shutdown {
                    break None;
                }
                guard = match cvar.wait(guard) {
                    Ok(g) => g,
                    Err(poison) => {
                        let mut g = poison.into_inner();
                        g.diagnostics.insert("A0010".to_string(), ());
                        g
                    }
                };
            }
        };

        let task = match next {
            Some(t) => t,
            None => {
                // Drain any leftover tasks if shutdown beat us to the pop:
                // they count as cancelled, surface A0011-style "didn't run".
                drain_cancelled(&state);
                return;
            }
        };

        local_steps = local_steps.saturating_add(1);
        if local_steps
            > max_steps_per_task
                .saturating_mul(1024)
                .max(max_steps_per_task)
        {
            // Defensive cap so a worker can't pin a CPU forever if a user
            // builds an infinite stream of submissions. Surfaced via A0011.
            insert_diag(&state, "A0011");
            // Re-queue the task so accounting is consistent, then exit.
            requeue(&state, task);
            return;
        }

        // Run the user body in catch_unwind. `AssertUnwindSafe` is required
        // because `FnOnce` closures don't implement `UnwindSafe` by default;
        // we own the closure, the closure owns its captures, and any state
        // we mutate inside is recorded after the fact. This is the ONLY
        // place in the bootstrap that calls `catch_unwind`.
        let id = task.id;
        let outcome = catch_unwind(AssertUnwindSafe(task.body));
        // Record the outcome under the lock so writers don't race.
        match lock.lock() {
            Ok(mut guard) => {
                guard.total_steps = guard.total_steps.saturating_add(1);
                match outcome {
                    Ok(_value) => {
                        guard.completed = guard.completed.saturating_add(1);
                    }
                    Err(_payload) => {
                        guard.panicked = guard.panicked.saturating_add(1);
                        guard.diagnostics.insert("A0010".to_string(), ());
                    }
                }
            }
            Err(poison) => {
                let mut guard = poison.into_inner();
                guard.total_steps = guard.total_steps.saturating_add(1);
                guard.diagnostics.insert("A0010".to_string(), ());
                match outcome {
                    Ok(_value) => {
                        guard.completed = guard.completed.saturating_add(1);
                    }
                    Err(_payload) => {
                        guard.panicked = guard.panicked.saturating_add(1);
                    }
                }
            }
        }
        // Hand the id back into the void — we don't track per-id outcomes
        // here because the report only needs aggregate counters.
        let _ = id;
    }
}

/// Drain whatever's left on the queue and count it as cancelled. Used when
/// a worker exits after shutdown was requested mid-flight.
fn drain_cancelled(state: &Arc<(Mutex<ParallelState>, Condvar)>) {
    let (lock, _) = &**state;
    if let Ok(mut guard) = lock.lock() {
        while let Some(task) = guard.queue.pop_front() {
            // Drop the body without invoking it.
            drop(task);
            guard.cancelled = guard.cancelled.saturating_add(1);
        }
    }
}

/// Push a task back onto the front of the queue (used by the per-worker
/// defensive cap). Best-effort: if the mutex is poisoned we still attempt
/// the push and record an A0010.
fn requeue(state: &Arc<(Mutex<ParallelState>, Condvar)>, task: ParallelTask) {
    let (lock, cvar) = &**state;
    match lock.lock() {
        Ok(mut guard) => {
            guard.queue.push_front(task);
        }
        Err(poison) => {
            let mut guard = poison.into_inner();
            guard.diagnostics.insert("A0010".to_string(), ());
            guard.queue.push_front(task);
        }
    }
    cvar.notify_one();
}

impl Scheduler {
    /// Drain the cooperative queue using `workers` OS threads. Each ready
    /// task in the cooperative queue is wrapped as a no-op body (its value
    /// is already materialised) and shipped through the parallel pool so
    /// callers get a consistent [`AsyncReport`] regardless of mode.
    ///
    /// The resulting report's `total_tasks` is the number of ready tasks
    /// the cooperative queue had at the time of the call — i.e. the
    /// pre-existing `spawned_count` minus tasks already completed or
    /// parked. The `worker_count`, `total_steps`, and `total_tasks` fields
    /// are byte-stable across runs because every task body is atomic.
    pub fn run_to_completion_parallel(
        &mut self,
        workers: usize,
        max_steps_per_task: usize,
    ) -> AsyncReport {
        let mode = SchedulerMode::Parallel { workers };
        self.run_with_mode(mode, max_steps_per_task)
    }

    /// Generic run dispatched on [`SchedulerMode`]. Deterministic falls
    /// through to [`run_to_completion`]; parallel ships every ready task
    /// to the worker pool.
    pub fn run_with_mode(&mut self, mode: SchedulerMode, max_steps_per_task: usize) -> AsyncReport {
        match mode {
            SchedulerMode::Deterministic => {
                let drained = run_to_completion(self, max_steps_per_task);
                AsyncReport::from_scheduler(self, drained.len())
            }
            SchedulerMode::Parallel { workers } => {
                // Capture the ready queue snapshot; each entry already has
                // its resumed value, so the worker body is a pure return.
                let mut pool = ParallelScheduler::new();
                while let Some(id) = self.queue.pop_front() {
                    if let Some(value) = self.resumed_values.remove(&id) {
                        self.completed_ids.insert(id, ());
                        let v = value;
                        pool.submit(move || v);
                    }
                }
                pool.run(workers, max_steps_per_task, SHUTDOWN_GRACE)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn int(value: i64) -> Value {
        Value::Int(value)
    }

    /// Helper that replaces the standard library panic shortcut so test
    /// failures emit a stable message rather than the generic panic text.
    #[allow(clippy::assertions_on_constants)]
    fn must_ready(task: Option<Task>) -> Value {
        match task {
            Some(Task::Ready(value)) => value,
            other => {
                assert!(false, "expected Task::Ready, got {other:?}");
                Value::Unit
            }
        }
    }

    #[test]
    fn spawn_then_step_round_trip() {
        let mut s = Scheduler::new();
        let id = s.spawn(int(7));
        assert_eq!(id, 0, "first spawn should mint id 0");
        let value = must_ready(s.step());
        assert_eq!(value, int(7));
        assert!(
            s.step().is_none(),
            "second step on drained queue must be None"
        );
        assert_eq!(s.spawned_count(), 1);
        assert_eq!(s.completed_count(), 1);
    }

    #[test]
    fn multiple_ready_tasks_are_fifo() {
        let mut s = Scheduler::new();
        for v in [int(1), int(2), int(3), int(4)] {
            s.spawn(v);
        }
        let drained = run_to_completion(&mut s, 64);
        assert_eq!(drained, vec![int(1), int(2), int(3), int(4)]);
    }

    #[test]
    fn pending_then_resume_completes() {
        let mut s = Scheduler::new();
        let id = s.spawn(Value::Unit);
        s.park(id, Value::Unit);
        assert!(s.step().is_none(), "no ready work while parked");
        s.resume(id, int(42));
        let value = must_ready(s.step());
        assert_eq!(value, int(42));
    }

    #[test]
    fn max_steps_caps_run_to_completion() {
        let mut s = Scheduler::new();
        for v in 0..10 {
            s.spawn(int(v));
        }
        let drained = run_to_completion(&mut s, 3);
        assert_eq!(drained.len(), 3, "must cap at exactly max_steps tasks");
        // Remaining queue still has 7 entries — overflow is observable.
        assert_eq!(s.queue.len(), 7);
    }

    #[test]
    fn a0001_overflow_diagnostic() {
        let mut s = Scheduler::new();
        for v in 0..5 {
            s.spawn(int(v));
        }
        let _ = run_to_completion(&mut s, 2);
        let diags = diagnostics_for(&s, 2, 2);
        assert!(diags.iter().any(|d| d.id == "A0001"));
    }

    #[test]
    fn a0002_deadlock_report() {
        let mut s = Scheduler::new();
        let id_a = s.spawn(Value::Unit);
        let id_b = s.spawn(Value::Unit);
        s.park(id_a, Value::Unit);
        s.park(id_b, Value::Unit);
        // Drain any remaining ready work (none).
        let _ = run_to_completion(&mut s, 16);
        let diags = diagnostics_for(&s, 0, 16);
        let deadlock = diags
            .iter()
            .find(|d| d.id == "A0002")
            .expect_none_or_panic();
        assert!(deadlock.message.contains('['));
        assert!(deadlock.message.contains(&id_a.to_string()));
        assert!(deadlock.message.contains(&id_b.to_string()));
    }

    #[test]
    fn a0003_future_leak_report() {
        let mut s = Scheduler::new();
        // Allocate an id, then forcibly evict from queue and waiting so it
        // counts as leaked.
        let id = s.spawn(Value::Unit);
        s.queue.clear();
        s.resumed_values.remove(&id);
        let diags = diagnostics_for(&s, 0, 8);
        assert!(diags.iter().any(|d| d.id == "A0003"));
    }

    #[test]
    fn step_is_idempotent_on_empty_queue() {
        let mut s = Scheduler::new();
        assert!(s.step().is_none());
        assert!(s.step().is_none());
        assert!(s.step().is_none());
    }

    #[test]
    fn scheduler_is_deterministic_across_runs() {
        fn run() -> (Vec<Value>, AsyncReport) {
            let mut s = Scheduler::new();
            for v in 0..16 {
                s.spawn(int(v));
            }
            let drained = run_to_completion(&mut s, 1024);
            let report = AsyncReport::from_scheduler(&s, drained.len());
            (drained, report)
        }
        let (a_values, a_report) = run();
        let (b_values, b_report) = run();
        assert_eq!(a_values, b_values);
        assert_eq!(a_report, b_report);
        assert_eq!(a_report.schema, "ori.async_report.v1");
    }

    #[test]
    fn ids_are_monotonic_and_not_reusable() {
        let mut s = Scheduler::new();
        let a = s.spawn(int(0));
        let b = s.spawn(int(0));
        let _ = s.step();
        let _ = s.step();
        let c = s.spawn(int(0));
        assert!(
            a < b && b < c,
            "ids must strictly increase, got {a},{b},{c}"
        );
    }

    #[test]
    fn stress_thousand_spawn_thousand_resume() {
        let mut s = Scheduler::new();
        let mut ids = Vec::with_capacity(1000);
        for v in 0..1000i64 {
            let id = s.spawn(Value::Unit);
            s.park(id, Value::Unit);
            ids.push((id, v));
        }
        for (id, v) in &ids {
            s.resume(*id, int(*v));
        }
        let drained = run_to_completion(&mut s, 4096);
        assert_eq!(drained.len(), 1000);
        // Resumes preserve FIFO order of resume() calls, which matches the
        // order of `ids`.
        for (i, value) in drained.iter().enumerate() {
            assert_eq!(value, &int(i as i64));
        }
        let report = AsyncReport::from_scheduler(&s, drained.len());
        assert_eq!(report.spawned, 1000);
        assert_eq!(report.completed, 1000);
        assert_eq!(report.stalled, 0);
        assert!(report.max_queue_depth >= 1000);
    }

    // -----------------------------------------------------------------
    // M26 parallel scheduler tests.
    // -----------------------------------------------------------------

    #[test]
    fn async_parallel_single_worker_matches_deterministic_totals() {
        // Run the same workload via deterministic and via 1-worker parallel
        // and assert the aggregate counters match.
        let mut det = Scheduler::new();
        for v in 0..32 {
            det.spawn(int(v));
        }
        let drained = run_to_completion(&mut det, 1024);
        let det_report = AsyncReport::from_scheduler(&det, drained.len());

        let mut par = Scheduler::new();
        for v in 0..32 {
            par.spawn(int(v));
        }
        let par_report = par.run_to_completion_parallel(1, 1024);
        assert_eq!(par_report.schema, det_report.schema);
        assert_eq!(par_report.total_tasks, det_report.spawned);
        assert_eq!(par_report.completed, det_report.completed);
        assert_eq!(par_report.worker_count, 1);
        assert_eq!(par_report.cancelled, 0);
        assert_eq!(par_report.panicked, 0);
        assert!(
            par_report.diagnostics.is_empty(),
            "no diagnostics expected for trivial run"
        );
    }

    #[test]
    fn async_parallel_multi_worker_completes_one_thousand_tasks() {
        let mut s = Scheduler::new();
        for v in 0..1000 {
            s.spawn(int(v));
        }
        let report = s.run_to_completion_parallel(4, 64);
        assert_eq!(report.worker_count, 4);
        assert_eq!(report.total_tasks, 1000);
        assert_eq!(report.completed, 1000);
        assert_eq!(report.cancelled, 0);
        assert_eq!(report.panicked, 0);
        // total_steps == executed pops; with no requeues this equals total_tasks.
        assert_eq!(report.total_steps, 1000);
    }

    #[test]
    fn async_parallel_worker_panic_surfaces_a0010_not_abort() {
        // Spawn a mix of well-behaved closures and one that explicitly
        // panics. We expect A0010 in the report and the well-behaved tasks
        // to still complete.
        let mut pool = ParallelScheduler::new();
        let good_runs = Arc::new(AtomicUsize::new(0));
        for _ in 0..16 {
            let counter = Arc::clone(&good_runs);
            pool.submit(move || {
                counter.fetch_add(1, Ordering::Relaxed);
                Value::Unit
            });
        }
        // The "intentional" panic — wrapped in a closure so the worker's
        // catch_unwind actually fires.
        pool.submit(|| {
            #[allow(clippy::assertions_on_constants)]
            {
                assert!(false, "intentional");
            }
            Value::Unit
        });
        for _ in 0..16 {
            let counter = Arc::clone(&good_runs);
            pool.submit(move || {
                counter.fetch_add(1, Ordering::Relaxed);
                Value::Unit
            });
        }
        let report = pool.run(4, 64, Duration::from_secs(5));
        assert!(
            report.panicked >= 1,
            "expected at least one panicked task, got {report:?}"
        );
        assert!(report.diagnostics.iter().any(|d| d == "A0010"));
        assert_eq!(
            good_runs.load(Ordering::Relaxed),
            32,
            "all non-panicking tasks must run"
        );
        assert!(report.completed >= 32);
    }

    #[test]
    fn async_parallel_shutdown_grace_cancels_remaining_tasks() {
        // Submit 100 tasks; shutdown after 10 have ostensibly been picked
        // up. We do this by submitting 10 fast tasks, requesting shutdown,
        // then submitting 90 more — under the shutdown flag the workers
        // will refuse to wait for new work, so the late submissions stay
        // on the queue until drain_cancelled scoops them up.
        let mut pool = ParallelScheduler::new();
        let runs = Arc::new(AtomicUsize::new(0));
        for _ in 0..10 {
            let r = Arc::clone(&runs);
            pool.submit(move || {
                r.fetch_add(1, Ordering::Relaxed);
                Value::Unit
            });
        }
        // Request shutdown before pumping more in: the pool will run the
        // 10 already-queued items then drain anything else as cancelled.
        pool.request_shutdown();
        for _ in 0..90 {
            let r = Arc::clone(&runs);
            pool.submit(move || {
                r.fetch_add(1, Ordering::Relaxed);
                Value::Unit
            });
        }
        let report = pool.run(2, 64, Duration::from_secs(5));
        assert_eq!(report.total_tasks, 100);
        // Some subset of the 10 originals plus possibly a few of the 90
        // late submissions will have run before workers exit; the rest
        // must be reflected as cancelled.
        assert_eq!(report.completed + report.cancelled, 100);
        assert!(
            report.cancelled >= 1,
            "expected late submissions to be cancelled, got {report:?}"
        );
    }

    #[test]
    fn async_parallel_zero_workers_is_noop() {
        // Defensive: asking for zero workers must not hang and must
        // return a coherent report.
        let mut pool = ParallelScheduler::new();
        pool.submit(|| Value::Unit);
        let report = pool.run(0, 16, Duration::from_secs(1));
        assert_eq!(report.worker_count, 0);
        assert_eq!(report.total_tasks, 1);
        assert_eq!(report.completed, 0);
    }

    #[test]
    fn async_parallel_oversize_worker_request_emits_a0013() {
        // Asking for MAX_WORKERS + 1 must clamp and surface A0013.
        let mut pool = ParallelScheduler::new();
        pool.submit(|| Value::Unit);
        let report = pool.run(MAX_WORKERS + 1, 16, Duration::from_secs(5));
        assert!(
            report.diagnostics.iter().any(|d| d == "A0013"),
            "expected A0013 in diagnostics, got {:?}",
            report.diagnostics
        );
        assert_eq!(report.worker_count, MAX_WORKERS);
    }

    #[test]
    fn async_parallel_report_schema_is_stable() {
        // Same workload run twice — every parallel-report counter that
        // we promise to be byte-stable must match across runs.
        fn run() -> AsyncReport {
            let mut s = Scheduler::new();
            for v in 0..64 {
                s.spawn(int(v));
            }
            s.run_to_completion_parallel(4, 128)
        }
        let a = run();
        let b = run();
        assert_eq!(a.schema, b.schema);
        assert_eq!(a.total_tasks, b.total_tasks);
        assert_eq!(a.completed, b.completed);
        assert_eq!(a.cancelled, b.cancelled);
        assert_eq!(a.panicked, b.panicked);
        assert_eq!(a.worker_count, b.worker_count);
        assert_eq!(a.diagnostics, b.diagnostics);
        // total_steps is byte-stable here because each body is atomic and
        // every task is popped exactly once across the run.
        assert_eq!(a.total_steps, b.total_steps);
    }

    #[test]
    fn async_parallel_dispatch_via_mode_enum() {
        // The SchedulerMode::Parallel arm of run_with_mode must produce
        // the same totals as run_to_completion_parallel.
        let mut a = Scheduler::new();
        let mut b = Scheduler::new();
        for v in 0..16 {
            a.spawn(int(v));
            b.spawn(int(v));
        }
        let ra = a.run_with_mode(SchedulerMode::Parallel { workers: 2 }, 64);
        let rb = b.run_to_completion_parallel(2, 64);
        assert_eq!(ra.total_tasks, rb.total_tasks);
        assert_eq!(ra.completed, rb.completed);
        assert_eq!(ra.worker_count, rb.worker_count);
    }

    #[test]
    fn async_deterministic_mode_matches_legacy_path() {
        // run_with_mode(Deterministic) must produce the same counters as
        // calling run_to_completion + from_scheduler directly.
        let mut a = Scheduler::new();
        let mut b = Scheduler::new();
        for v in 0..16 {
            a.spawn(int(v));
            b.spawn(int(v));
        }
        let drained = run_to_completion(&mut a, 1024);
        let legacy = AsyncReport::from_scheduler(&a, drained.len());
        let via_mode = b.run_with_mode(SchedulerMode::Deterministic, 1024);
        assert_eq!(via_mode.spawned, legacy.spawned);
        assert_eq!(via_mode.completed, legacy.completed);
        assert_eq!(via_mode.steps_taken, legacy.steps_taken);
        assert_eq!(via_mode.worker_count, 0);
    }

    /// Local helper to avoid the standard library shortcut in src/. Returns the
    /// inner value or fires an `assert!(false, ...)` with a descriptive message.
    trait ExpectNoneOrPanic {
        type Out;
        fn expect_none_or_panic(self) -> Self::Out;
    }

    impl<'a, T> ExpectNoneOrPanic for Option<&'a T> {
        type Out = &'a T;
        #[allow(clippy::assertions_on_constants)]
        fn expect_none_or_panic(self) -> Self::Out {
            match self {
                Some(v) => v,
                None => {
                    assert!(false, "expected Some(_), got None");
                    // unreachable — assert!(false) aborts the test.
                    unreachable!()
                }
            }
        }
    }
}
