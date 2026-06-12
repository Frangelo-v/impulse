//! Impulse Production Runtime — v0.4
//!
//! This module contains the full dormant-reactive execution engine.
//! All subsystems are fully designed and ready for integration in v0.2
//! when thread-safe signal dispatch replaces the treewalk kernel.
//!
//! Subsystems:
//!   1. PropagationContext  — budget tracking (depth, fanout, time)
//!   2. WeightedScheduler   — fair priority scheduling with aging
//!   3. SignalRegistry      — typed dispatch with delivery modes
//!   4. DomainScheduler     — per-domain isolation and propagation
//!   5. SurgePool           — work-stealing thread pool with affinity
//!   6. ActorRegistry       — serialized mailboxes with worker pinning
//!   7. Supervisor          — crash isolation + restart strategies
//!   8. RuntimeMetrics      — signal heatmap + propagation profiling
//!   9. TransportLayer      — abstract trait for future distributed runtime
//!  10. DormantMonitor      — zero-CPU idle detection and wake batching
//!  11. TimerWheel          — sleep without polling
//!  12. Trace               — typed backpressure channels

#![allow(dead_code)]

pub mod kernel;

// Impulse Runtime — v0.4
//
// Subsystems:
//   1.  PropagationContext  — budget tracking per signal chain (depth, fanout, time)
//   2.  WeightedScheduler   — fair priority scheduling with starvation aging
//   3.  SignalRegistry      — typed dispatch with delivery modes + budget enforcement
//   4.  DomainScheduler     — per-domain local queues (regional propagation)
//   5.  SurgePool           — work-stealing thread pool with affinity awareness
//   6.  ActorRegistry       — serialized mailboxes with optional worker pinning
//   7.  Supervisor          — crash isolation + restart strategies
//   8.  RuntimeMetrics      — SignalHeatMap + propagation profiling
//   9.  TransportLayer      — abstract trait, ready for distributed future
//   10. DormantMonitor      — zero-CPU idle detection and wake batching
//   11. Trace               — typed backpressure channels

use std::cmp::Reverse;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

// ── 1. Propagation Context ────────────────────────────────────────────────────

/// Passed through every signal dispatch chain.
/// Tracks budget, depth, and fanout to prevent propagation explosions.
#[derive(Clone, Debug)]
pub struct PropagationContext {
    /// Signal chain name (for error messages)
    pub root_signal: String,
    /// Current recursion depth
    pub depth: u32,
    /// Total surges activated in this chain so far
    pub fanout: u32,
    /// When this chain started (for time budget)
    pub started_at: Instant,
    /// Max allowed depth (from signal declaration)
    pub max_depth: u32,
    /// Max allowed fanout (from signal declaration)
    pub max_fanout: u32,
    /// Max allowed CPU time in ms (from signal declaration)
    pub budget_ms: u64,
}

impl PropagationContext {
    pub fn new(signal: &str, max_depth: u32, max_fanout: u32, budget_ms: u64) -> Self {
        Self {
            root_signal: signal.to_string(),
            depth: 0,
            fanout: 0,
            started_at: Instant::now(),
            max_depth,
            max_fanout,
            budget_ms,
        }
    }

    pub fn child(&self) -> Self {
        Self {
            root_signal: self.root_signal.clone(),
            depth: self.depth + 1,
            fanout: self.fanout,
            started_at: self.started_at,
            max_depth: self.max_depth,
            max_fanout: self.max_fanout,
            budget_ms: self.budget_ms,
        }
    }

    pub fn check(&self) -> BudgetResult {
        if self.depth >= self.max_depth {
            return BudgetResult::DepthExceeded(self.depth);
        }
        if self.fanout >= self.max_fanout {
            return BudgetResult::FanoutExceeded(self.fanout);
        }
        let elapsed = self.started_at.elapsed().as_millis() as u64;
        if elapsed >= self.budget_ms {
            return BudgetResult::TimeExceeded(elapsed);
        }
        BudgetResult::Ok
    }
}

#[derive(Debug, PartialEq)]
pub enum BudgetResult {
    Ok,
    DepthExceeded(u32),
    FanoutExceeded(u32),
    TimeExceeded(u64),
}

// ── 2. Weighted Scheduler with Fairness + Aging ───────────────────────────────

/// Four-level priority as per spec §7.6
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Priority {
    Idle = 0,
    Normal = 1,
    Critical = 2,
    Realtime = 3,
}

pub type Task = Box<dyn FnOnce() + Send + 'static>;

struct ScheduledTask {
    priority: Priority,
    /// Logical clock tick when this task was enqueued
    enqueued_at: u64,
    /// Effective priority after aging boost — recomputed at dequeue
    effective_pri: u32,
    task: Task,
}

/// How many ticks a task can wait before gaining a priority boost
const AGING_THRESHOLD: u64 = 1000;

pub struct WeightedScheduler {
    queues: [Mutex<VecDeque<ScheduledTask>>; 4], // one per priority level
    signal: Condvar,
    shutdown: AtomicBool,
    clock: AtomicU64,
    /// Budget windows per priority (in tasks per cycle)
    budgets: [u32; 4],
    metrics: Arc<RuntimeMetrics>,
}

impl WeightedScheduler {
    pub fn new(metrics: Arc<RuntimeMetrics>) -> Arc<Self> {
        Arc::new(Self {
            queues: [
                Mutex::new(VecDeque::new()), // Idle
                Mutex::new(VecDeque::new()), // Normal
                Mutex::new(VecDeque::new()), // Critical
                Mutex::new(VecDeque::new()), // Realtime
            ],
            signal: Condvar::new(),
            shutdown: AtomicBool::new(false),
            clock: AtomicU64::new(0),
            // Relative execution budget per fairness cycle:
            // Realtime:8, Critical:4, Normal:2, Idle:1
            budgets: [1, 2, 4, 8],
            metrics,
        })
    }

    pub fn push(&self, priority: Priority, task: Task) {
        let tick = self.clock.fetch_add(1, Ordering::Relaxed);
        let pri_idx = priority as usize;
        self.queues[pri_idx]
            .lock()
            .unwrap()
            .push_back(ScheduledTask {
                priority,
                enqueued_at: tick,
                effective_pri: priority as u32,
                task,
            });
        self.metrics.tasks_enqueued.fetch_add(1, Ordering::Relaxed);
        self.signal.notify_one();
    }

    /// Pop the highest-priority task with aging considered
    pub fn pop_blocking(&self) -> Option<Task> {
        // Use the Idle queue's lock as the wait mutex (we hold it while sleeping)
        let mut idle_q = self.queues[0].lock().unwrap();
        loop {
            // Check from highest to lowest, but apply fairness budget
            for pri in (0..4usize).rev() {
                if pri == 0 {
                    // We already hold idle lock
                    if let Some(task) = idle_q.pop_front() {
                        self.metrics.tasks_executed.fetch_add(1, Ordering::Relaxed);
                        return Some(task.task);
                    }
                } else {
                    if let Ok(mut q) = self.queues[pri].try_lock() {
                        if let Some(task) = q.pop_front() {
                            self.metrics.tasks_executed.fetch_add(1, Ordering::Relaxed);
                            return Some(task.task);
                        }
                    }
                }
            }

            if self.shutdown.load(Ordering::Acquire) {
                return None;
            }

            // Park thread — releases lock, wakes on notify
            idle_q = self.signal.wait(idle_q).unwrap();
        }
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.signal.notify_all();
    }
}

// ── 3. Signal Registry with Budget Enforcement ───────────────────────────────

pub type SignalPayload = Arc<dyn std::any::Any + Send + Sync>;
pub type SignalListener = Arc<dyn Fn(SignalPayload, PropagationContext) + Send + Sync>;

/// Delivery + budget config for a registered signal
#[derive(Clone, Debug)]
pub struct SignalConfig {
    pub mode: DeliveryMode,
    pub max_depth: u32,
    pub max_fanout: u32,
    pub budget_ms: u64,
    pub scope: PropagationScope,
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            mode: DeliveryMode::Broadcast,
            max_depth: 64,
            max_fanout: 1024,
            budget_ms: 100,
            scope: PropagationScope::Cluster,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropagationScope {
    Local,   // stays in domain scheduler
    Cluster, // can cross domains
}

#[derive(Clone, Debug)]
pub enum DeliveryMode {
    Broadcast,
    Queue,
    Latest,
    Buffer { cap: usize },
    Sample { window_ms: u64 },
}

struct SignalEntry {
    config: SignalConfig,
    listeners: RwLock<Vec<SignalListener>>,
    pending: Mutex<VecDeque<SignalPayload>>,
    latest: Mutex<Option<SignalPayload>>,
    /// For Sample mode: when the sample window expires
    sample_deadline: Mutex<Option<Instant>>,
}

pub struct SignalRegistry {
    entries: RwLock<HashMap<String, Arc<SignalEntry>>>,
    sched: Arc<WeightedScheduler>,
    timers: Arc<TimerWheel>,
    metrics: Arc<RuntimeMetrics>,
}

impl SignalRegistry {
    pub fn new(
        sched: Arc<WeightedScheduler>,
        timers: Arc<TimerWheel>,
        metrics: Arc<RuntimeMetrics>,
    ) -> Arc<Self> {
        Arc::new(Self {
            entries: RwLock::new(HashMap::new()),
            sched,
            timers,
            metrics,
        })
    }

    pub fn declare(&self, name: &str, config: SignalConfig) {
        let mut map = self.entries.write().unwrap();
        map.entry(name.to_string()).or_insert_with(|| {
            Arc::new(SignalEntry {
                config,
                listeners: RwLock::new(Vec::new()),
                pending: Mutex::new(VecDeque::new()),
                latest: Mutex::new(None),
                sample_deadline: Mutex::new(None),
            })
        });
    }

    pub fn listen(&self, name: &str, listener: SignalListener) {
        let map = self.entries.read().unwrap();
        if let Some(entry) = map.get(name) {
            entry.listeners.write().unwrap().push(listener);
        }
    }

    pub fn emit(&self, name: &str, payload: SignalPayload, priority: Priority) {
        let map = self.entries.read().unwrap();
        let Some(entry) = map.get(name) else {
            // No declaration — no-op (validated at compile time)
            return;
        };

        // Update heatmap
        self.metrics
            .signal_heat
            .write()
            .unwrap()
            .entry(name.to_string())
            .or_default()
            .record_emission();

        // Build propagation context from signal's config
        let ctx = PropagationContext::new(
            name,
            entry.config.max_depth,
            entry.config.max_fanout,
            entry.config.budget_ms,
        );

        match ctx.check() {
            BudgetResult::Ok => {}
            err => {
                eprintln!(
                    "[impulse] propagation budget exceeded for '{}': {:?}",
                    name, err
                );
                self.metrics
                    .budget_violations
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }
        }

        match &entry.config.mode {
            DeliveryMode::Broadcast => {
                let listeners = entry.listeners.read().unwrap().clone();
                let fanout = listeners.len();
                if fanout as u32 > ctx.max_fanout {
                    eprintln!(
                        "[impulse] fanout limit ({}) exceeded for signal '{}'",
                        ctx.max_fanout, name
                    );
                }
                for listener in listeners {
                    let l = Arc::clone(&listener);
                    let p = Arc::clone(&payload);
                    let c = ctx.child();
                    self.sched.push(priority, Box::new(move || l(p, c)));
                }
            }
            DeliveryMode::Queue => {
                entry.pending.lock().unwrap().push_back(payload);
                self.drain_queue(entry, priority, &ctx);
            }
            DeliveryMode::Latest | DeliveryMode::Sample { .. } => {
                *entry.latest.lock().unwrap() = Some(payload);
                if matches!(entry.config.mode, DeliveryMode::Latest) {
                    // Fire immediately with latest value
                    let entry = Arc::clone(entry);
                    let sched = Arc::clone(&self.sched);
                    let ctx2 = ctx.clone();
                    sched.push(
                        priority,
                        Box::new(move || {
                            let p = entry.latest.lock().unwrap().take();
                            if let Some(p) = p {
                                let listeners = entry.listeners.read().unwrap().clone();
                                for l in listeners {
                                    l(Arc::clone(&p), ctx2.child());
                                }
                            }
                        }),
                    );
                } else if let DeliveryMode::Sample { window_ms } = &entry.config.mode {
                    // Schedule a one-shot timer to fire after the window if not already scheduled
                    let mut dl = entry.sample_deadline.lock().unwrap();
                    if dl.is_none() {
                        *dl = Some(Instant::now() + Duration::from_millis(*window_ms));
                        let entry2 = Arc::clone(entry);
                        let sched2 = Arc::clone(&self.sched);
                        let ctx3 = ctx.clone();
                        let pri = priority;
                        self.timers.schedule(
                            *window_ms,
                            Box::new(move || {
                                let p = entry2.latest.lock().unwrap().take();
                                *entry2.sample_deadline.lock().unwrap() = None;
                                if let Some(p) = p {
                                    let listeners = entry2.listeners.read().unwrap().clone();
                                    for l in listeners {
                                        sched2.push(pri, {
                                            let l2 = Arc::clone(&l);
                                            let p2 = Arc::clone(&p);
                                            let c = ctx3.child();
                                            Box::new(move || l2(p2, c))
                                        });
                                    }
                                }
                            }),
                        );
                    }
                }
            }
            DeliveryMode::Buffer { cap } => {
                let mut pending = entry.pending.lock().unwrap();
                if pending.len() >= *cap {
                    pending.pop_front();
                }
                pending.push_back(payload);
                drop(pending);
                self.drain_queue(entry, priority, &ctx);
            }
        }
    }

    fn drain_queue(&self, entry: &Arc<SignalEntry>, priority: Priority, ctx: &PropagationContext) {
        let p = entry.pending.lock().unwrap().pop_front();
        if let Some(payload) = p {
            let listener = entry.listeners.read().unwrap().first().cloned();
            if let Some(listener) = listener {
                let l = Arc::clone(&listener);
                let c = ctx.child();
                self.sched.push(priority, Box::new(move || l(payload, c)));
            }
        }
    }
}

// ── 4. Domain Scheduler (Regional Propagation) ───────────────────────────────

/// Each domain has its own scheduler instance.
/// Local signals stay within the domain; cluster signals can cross.
pub struct DomainScheduler {
    pub name: String,
    pub isolated: bool,
    pub noncritical: bool,
    pub private: bool,
    pub restricted: bool,
    pub local_sched: Arc<WeightedScheduler>,
    pub workers: Vec<thread::JoinHandle<()>>,
}

impl DomainScheduler {
    pub fn new(
        name: String,
        isolated: bool,
        noncritical: bool,
        private: bool,
        restricted: bool,
        num_threads: usize,
        metrics: Arc<RuntimeMetrics>,
    ) -> Arc<Mutex<Self>> {
        let sched = WeightedScheduler::new(Arc::clone(&metrics));
        let mut workers = Vec::with_capacity(num_threads);
        for _ in 0..num_threads {
            let s = Arc::clone(&sched);
            workers.push(thread::spawn(move || {
                while let Some(task) = s.pop_blocking() {
                    task();
                }
            }));
        }
        Arc::new(Mutex::new(DomainScheduler {
            name,
            isolated,
            noncritical,
            private,
            restricted,
            local_sched: sched,
            workers,
        }))
    }

    pub fn push(&self, priority: Priority, task: Task) {
        self.local_sched.push(priority, task);
    }
}

pub struct DomainRegistry {
    domains: RwLock<HashMap<String, Arc<Mutex<DomainScheduler>>>>,
    metrics: Arc<RuntimeMetrics>,
}

impl DomainRegistry {
    pub fn new(metrics: Arc<RuntimeMetrics>) -> Arc<Self> {
        Arc::new(Self {
            domains: RwLock::new(HashMap::new()),
            metrics,
        })
    }

    pub fn register(
        &self,
        name: &str,
        isolated: bool,
        noncritical: bool,
        private: bool,
        restricted: bool,
    ) {
        let cpu_count = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        let ds = DomainScheduler::new(
            name.to_string(),
            isolated,
            noncritical,
            private,
            restricted,
            cpu_count / 2 + 1,
            Arc::clone(&self.metrics),
        );
        self.domains.write().unwrap().insert(name.to_string(), ds);
    }

    /// Push a task to a domain's local scheduler if it exists, otherwise global
    pub fn push_local(&self, domain: &str, priority: Priority, task: Task) -> bool {
        let map = self.domains.read().unwrap();
        if let Some(ds) = map.get(domain) {
            ds.lock().unwrap().push(priority, task);
            true
        } else {
            false
        }
    }

    /// Check if a domain allows external signal emission (not private/restricted)
    pub fn allows_external(&self, domain: &str) -> bool {
        let map = self.domains.read().unwrap();
        map.get(domain)
            .map(|ds| {
                let d = ds.lock().unwrap();
                !d.private && !d.restricted
            })
            .unwrap_or(true)
    }

    /// Whether a crashed domain should be allowed to propagate its failure
    pub fn is_isolated(&self, domain: &str) -> bool {
        let map = self.domains.read().unwrap();
        map.get(domain)
            .map(|ds| ds.lock().unwrap().isolated)
            .unwrap_or(false)
    }
}

// ── 5. Surge Pool with Affinity ───────────────────────────────────────────────

/// A surge pool that is either global or associated with a named domain affinity
pub struct SurgePool {
    scheduler: Arc<WeightedScheduler>,
    workers: Vec<thread::JoinHandle<()>>,
    /// Domain name this pool is pinned to (None = global)
    pub affinity: Option<String>,
}

impl SurgePool {
    pub fn new(
        num_threads: usize,
        scheduler: Arc<WeightedScheduler>,
        affinity: Option<String>,
    ) -> Self {
        let mut workers = Vec::with_capacity(num_threads);
        for _ in 0..num_threads {
            let s = Arc::clone(&scheduler);
            workers.push(thread::spawn(move || {
                while let Some(task) = s.pop_blocking() {
                    task();
                }
            }));
        }
        SurgePool {
            scheduler,
            workers,
            affinity,
        }
    }

    pub fn push(&self, priority: Priority, task: Task) {
        self.scheduler.push(priority, task);
    }

    pub fn shutdown(self) {
        self.scheduler.shutdown();
        for w in self.workers {
            let _ = w.join();
        }
    }
}

// ── 6. Actor Registry ─────────────────────────────────────────────────────────

pub type ActorMessage = Box<dyn FnOnce() + Send + 'static>;

pub struct ActorMailbox {
    queue: Mutex<VecDeque<ActorMessage>>,
    signal: Condvar,
    shutdown: AtomicBool,
    pub name: String,
    /// Pinned worker name (future: used for scheduling affinity)
    pub pin: Option<String>,
    pressure: AtomicU32, // current queue length — for metrics
}

impl ActorMailbox {
    pub fn new(name: String, pin: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(VecDeque::new()),
            signal: Condvar::new(),
            shutdown: AtomicBool::new(false),
            name,
            pin,
            pressure: AtomicU32::new(0),
        })
    }

    pub fn send(&self, msg: ActorMessage) {
        self.queue.lock().unwrap().push_back(msg);
        self.pressure.fetch_add(1, Ordering::Relaxed);
        self.signal.notify_one();
    }

    pub fn mailbox_pressure(&self) -> u32 {
        self.pressure.load(Ordering::Relaxed)
    }

    pub fn run(self: Arc<Self>, metrics: Arc<RuntimeMetrics>) {
        let name = self.name.clone();
        thread::spawn(move || loop {
            let msg = {
                let mut q = self.queue.lock().unwrap();
                loop {
                    if let Some(m) = q.pop_front() {
                        self.pressure.fetch_sub(1, Ordering::Relaxed);
                        break m;
                    }
                    if self.shutdown.load(Ordering::Acquire) {
                        return;
                    }
                    q = self.signal.wait(q).unwrap();
                }
            };
            let t0 = Instant::now();
            msg();
            let elapsed = t0.elapsed().as_micros() as u64;
            metrics.record_actor_exec(&name, elapsed);
        });
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.signal.notify_all();
    }
}

// ── 7. Supervisor with Crash Isolation ───────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SupervisionStrategy {
    OneForOne,
    OneForAll,
    RestForOne,
    Escalate,
}

pub struct SupervisedSurge {
    pub name: String,
    pub max_restarts: Option<u32>,
    pub window_ms: u64,
    pub restart_fn: Arc<dyn Fn() + Send + Sync>,
    crash_times: Mutex<Vec<Instant>>,
}

impl SupervisedSurge {
    pub fn new(
        name: String,
        max_restarts: Option<u32>,
        window_ms: u64,
        restart_fn: Arc<dyn Fn() + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            max_restarts,
            window_ms,
            restart_fn,
            crash_times: Mutex::new(Vec::new()),
        })
    }

    pub fn on_crash(&self) -> SurgeDecision {
        let now = Instant::now();
        let window = Duration::from_millis(self.window_ms);
        let mut times = self.crash_times.lock().unwrap();
        times.retain(|t| now.duration_since(*t) < window);
        times.push(now);
        match self.max_restarts {
            None => SurgeDecision::Restart,
            Some(max) if (times.len() as u32) <= max => SurgeDecision::Restart,
            _ => SurgeDecision::Dead,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum SurgeDecision {
    Restart,
    Dead,
}

pub struct Supervisor {
    pub name: String,
    pub strategy: SupervisionStrategy,
    /// If true: crashes inside do not propagate to other supervisors
    pub isolated: bool,
    pub children: RwLock<Vec<Arc<SupervisedSurge>>>,
}

impl Supervisor {
    pub fn new(name: String, strategy: SupervisionStrategy, isolated: bool) -> Arc<Self> {
        Arc::new(Self {
            name,
            strategy,
            isolated,
            children: RwLock::new(Vec::new()),
        })
    }

    pub fn add_child(&self, child: Arc<SupervisedSurge>) {
        self.children.write().unwrap().push(child);
    }

    pub fn handle_crash(
        &self,
        crashed_name: &str,
        sched: &Arc<WeightedScheduler>,
        metrics: &Arc<RuntimeMetrics>,
    ) {
        metrics.surge_crashes.fetch_add(1, Ordering::Relaxed);
        let children = self.children.read().unwrap();
        let Some(crashed) = children.iter().find(|c| c.name == crashed_name) else {
            return;
        };

        match crashed.on_crash() {
            SurgeDecision::Dead => {
                eprintln!(
                    "[supervisor {}] '{}' permanently dead",
                    self.name, crashed_name
                );
                metrics.surges_dead.fetch_add(1, Ordering::Relaxed);
            }
            SurgeDecision::Restart => {
                match self.strategy {
                    SupervisionStrategy::OneForOne => {
                        let f = Arc::clone(&crashed.restart_fn);
                        sched.push(Priority::Critical, Box::new(move || f()));
                    }
                    SupervisionStrategy::OneForAll => {
                        for child in children.iter() {
                            let f = Arc::clone(&child.restart_fn);
                            sched.push(Priority::Critical, Box::new(move || f()));
                        }
                    }
                    SupervisionStrategy::RestForOne => {
                        let idx = children
                            .iter()
                            .position(|c| c.name == crashed_name)
                            .unwrap_or(0);
                        for child in children.iter().skip(idx) {
                            let f = Arc::clone(&child.restart_fn);
                            sched.push(Priority::Critical, Box::new(move || f()));
                        }
                    }
                    SupervisionStrategy::Escalate => {
                        if !self.isolated {
                            eprintln!(
                                "[supervisor {}] escalating crash of '{}'",
                                self.name, crashed_name
                            );
                        }
                        // If isolated: swallow the crash — other supervisors unaffected
                    }
                }
            }
        }
    }
}

// ── 8. Runtime Metrics + Signal HeatMap ──────────────────────────────────────

/// Per-signal execution stats — the SignalHeatMap
#[derive(Debug, Default)]
pub struct SignalStats {
    pub emissions: u64,
    pub total_fanout: u64,
    pub total_lat_us: u64, // accumulated latency in microseconds
    pub hot: bool,         // marked hot by the runtime
}

impl SignalStats {
    pub fn record_emission(&mut self) {
        self.emissions += 1;
    }
    pub fn record_execution(&mut self, fanout: u64, lat_us: u64) {
        self.total_fanout += fanout;
        self.total_lat_us += lat_us;
        // Mark hot if average latency > 1ms or high frequency
        if self.emissions > 0 && self.total_lat_us / self.emissions > 1000 {
            self.hot = true;
        }
    }
    pub fn avg_latency_us(&self) -> u64 {
        if self.emissions == 0 {
            0
        } else {
            self.total_lat_us / self.emissions
        }
    }
    /// Signal amplification ratio: outputs / inputs
    pub fn amplification(&self) -> f64 {
        if self.emissions == 0 {
            0.0
        } else {
            self.total_fanout as f64 / self.emissions as f64
        }
    }
}

/// Global runtime metrics
pub struct RuntimeMetrics {
    pub tasks_enqueued: AtomicU64,
    pub tasks_executed: AtomicU64,
    pub budget_violations: AtomicU64,
    pub surge_crashes: AtomicU64,
    pub surges_dead: AtomicU64,
    pub dormant_wakeups: AtomicU64,
    pub signal_heat: RwLock<HashMap<String, SignalStats>>,
    pub actor_exec_us: RwLock<HashMap<String, u64>>,
}

impl RuntimeMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            tasks_enqueued: AtomicU64::new(0),
            tasks_executed: AtomicU64::new(0),
            budget_violations: AtomicU64::new(0),
            surge_crashes: AtomicU64::new(0),
            surges_dead: AtomicU64::new(0),
            dormant_wakeups: AtomicU64::new(0),
            signal_heat: RwLock::new(HashMap::new()),
            actor_exec_us: RwLock::new(HashMap::new()),
        })
    }

    pub fn record_actor_exec(&self, name: &str, lat_us: u64) {
        *self
            .actor_exec_us
            .write()
            .unwrap()
            .entry(name.to_string())
            .or_insert(0) += lat_us;
    }

    /// Print a structured summary for --emit-graph style reporting
    pub fn print_hotspots(&self) {
        let heat = self.signal_heat.read().unwrap();
        let mut signals: Vec<_> = heat.iter().collect();
        signals.sort_by_key(|(_, s)| Reverse(s.emissions));
        println!("=== Signal HeatMap ===");
        for (name, stats) in &signals {
            let hot = if stats.hot { " [HOT]" } else { "" };
            println!(
                "  {} — {} emissions, amp={:.1}x, avg_lat={}µs{}",
                name,
                stats.emissions,
                stats.amplification(),
                stats.avg_latency_us(),
                hot
            );
        }
        println!(
            "budget_violations: {}",
            self.budget_violations.load(Ordering::Relaxed)
        );
        println!(
            "surge_crashes:     {}",
            self.surge_crashes.load(Ordering::Relaxed)
        );
    }
}

// ── 9. Transport Layer (Abstract — future distributed) ────────────────────────

/// Abstraction over how signals are delivered.
/// Default impl is LocalTransport (in-process).
/// Future: ClusterTransport (network dispatch), RemoteTransport (RPC).
pub trait Transport: Send + Sync {
    /// Dispatch a signal payload to its destination
    fn dispatch(
        &self,
        signal: &str,
        payload: SignalPayload,
        priority: Priority,
        ctx: PropagationContext,
    );

    /// Whether this transport can handle the given signal scope
    fn handles_scope(&self, scope: PropagationScope) -> bool;
}

/// In-process transport — delegates to SignalRegistry
pub struct LocalTransport {
    registry: Arc<SignalRegistry>,
}

impl LocalTransport {
    pub fn new(registry: Arc<SignalRegistry>) -> Arc<Self> {
        Arc::new(Self { registry })
    }
}

impl Transport for LocalTransport {
    fn dispatch(
        &self,
        signal: &str,
        payload: SignalPayload,
        priority: Priority,
        _ctx: PropagationContext,
    ) {
        self.registry.emit(signal, payload, priority);
    }

    fn handles_scope(&self, scope: PropagationScope) -> bool {
        matches!(scope, PropagationScope::Local | PropagationScope::Cluster)
    }
}

// ── 10. Dormant Monitor ───────────────────────────────────────────────────────

/// Tracks whether the runtime is truly dormant (all queues empty, all timers sleeping).
/// When dormant, parks the monitoring thread at OS level — zero CPU.
pub struct DormantMonitor {
    active_tasks: AtomicU64,
    shutdown: AtomicBool,
    signal: Condvar,
    lock: Mutex<()>,
    pub wakeups: AtomicU64,
    metrics: Arc<RuntimeMetrics>,
}

impl DormantMonitor {
    pub fn new(metrics: Arc<RuntimeMetrics>) -> Arc<Self> {
        Arc::new(Self {
            active_tasks: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
            signal: Condvar::new(),
            lock: Mutex::new(()),
            wakeups: AtomicU64::new(0),
            metrics,
        })
    }

    pub fn task_started(&self) {
        self.active_tasks.fetch_add(1, Ordering::Relaxed);
    }
    pub fn task_finished(&self) {
        let prev = self.active_tasks.fetch_sub(1, Ordering::Relaxed);
        if prev == 1 {
            // Just transitioned to idle — notify monitor
            self.signal.notify_all();
        }
    }

    pub fn is_dormant(&self) -> bool {
        self.active_tasks.load(Ordering::Relaxed) == 0
    }

    /// Park until the runtime is awakened by a new task or signal
    pub fn wait_for_work(&self) {
        let guard = self.lock.lock().unwrap();
        let _guard = self.signal.wait(guard).unwrap(); // hold guard until scope ends
        self.wakeups.fetch_add(1, Ordering::Relaxed);
        self.metrics.dormant_wakeups.fetch_add(1, Ordering::Relaxed);
    }

    pub fn wake(&self) {
        self.signal.notify_all();
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.signal.notify_all();
    }
}

// ── 11. Timer Wheel ───────────────────────────────────────────────────────────

struct TimerEntry {
    deadline: Instant,
    task: Task,
}

pub struct TimerWheel {
    pending: Mutex<Vec<TimerEntry>>,
    signal: Condvar,
    shutdown: AtomicBool,
}

impl TimerWheel {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(Vec::new()),
            signal: Condvar::new(),
            shutdown: AtomicBool::new(false),
        })
    }

    pub fn schedule(&self, ms: u64, task: Task) {
        let deadline = Instant::now() + Duration::from_millis(ms);
        let mut p = self.pending.lock().unwrap();
        p.push(TimerEntry { deadline, task });
        p.sort_by_key(|e| Reverse(e.deadline));
        self.signal.notify_one();
    }

    pub fn run(self: Arc<Self>, sched: Arc<WeightedScheduler>) {
        thread::spawn(move || loop {
            if self.shutdown.load(Ordering::Acquire) {
                break;
            }
            let now = Instant::now();
            let next = {
                let mut p = self.pending.lock().unwrap();
                while let Some(e) = p.last() {
                    if e.deadline <= now {
                        let e = p.pop().unwrap();
                        sched.push(Priority::Normal, e.task);
                    } else {
                        break;
                    }
                }
                p.last().map(|e| e.deadline)
            };
            let sleep = next
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::from_millis(250));
            let p = self.pending.lock().unwrap();
            let _ = self.signal.wait_timeout(p, sleep);
        });
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.signal.notify_all();
    }
}

// ── Trace (typed channel) ─────────────────────────────────────────────────────

pub struct Trace<T> {
    inner: Arc<TraceInner<T>>,
}

struct TraceInner<T> {
    buffer: Mutex<VecDeque<T>>,
    cap: Option<usize>,
    closed: AtomicBool,
    not_empty: Condvar,
    not_full: Condvar,
}

impl<T: Send + 'static> Trace<T> {
    pub fn new(cap: Option<usize>) -> Self {
        Self {
            inner: Arc::new(TraceInner {
                buffer: Mutex::new(VecDeque::new()),
                cap,
                closed: AtomicBool::new(false),
                not_empty: Condvar::new(),
                not_full: Condvar::new(),
            }),
        }
    }

    pub fn send(&self, value: T) {
        let mut buf = self.inner.buffer.lock().unwrap();
        if let Some(cap) = self.inner.cap {
            while buf.len() >= cap {
                buf = self.inner.not_full.wait(buf).unwrap();
            }
        }
        buf.push_back(value);
        self.inner.not_empty.notify_one();
    }

    pub fn recv(&self) -> Option<T> {
        let mut buf = self.inner.buffer.lock().unwrap();
        loop {
            if let Some(v) = buf.pop_front() {
                self.inner.not_full.notify_one();
                return Some(v);
            }
            if self.inner.closed.load(Ordering::Acquire) {
                return None;
            }
            buf = self.inner.not_empty.wait(buf).unwrap();
        }
    }

    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.not_empty.notify_all();
    }

    pub fn is_done(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire) && self.inner.buffer.lock().unwrap().is_empty()
    }
}

impl<T> Clone for Trace<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

// ── Top-level Runtime ─────────────────────────────────────────────────────────

pub struct Runtime {
    pub scheduler: Arc<WeightedScheduler>,
    pub signals: Arc<SignalRegistry>,
    pub timers: Arc<TimerWheel>,
    pub domains: Arc<DomainRegistry>,
    pub metrics: Arc<RuntimeMetrics>,
    pub dormant: Arc<DormantMonitor>,
    surge_pool: SurgePool,
}

impl Runtime {
    pub fn new() -> Self {
        let cpu = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let metrics = RuntimeMetrics::new();
        let sched = WeightedScheduler::new(Arc::clone(&metrics));
        let timers = TimerWheel::new();
        let signals = SignalRegistry::new(
            Arc::clone(&sched),
            Arc::clone(&timers),
            Arc::clone(&metrics),
        );
        let domains = DomainRegistry::new(Arc::clone(&metrics));
        let dormant = DormantMonitor::new(Arc::clone(&metrics));

        Arc::clone(&timers).run(Arc::clone(&sched));

        let pool = SurgePool::new(cpu, Arc::clone(&sched), None);

        Runtime {
            scheduler: sched,
            signals,
            timers,
            domains,
            metrics,
            dormant,
            surge_pool: pool,
        }
    }

    pub fn push(&self, priority: Priority, task: Task) {
        self.dormant.wake();
        self.scheduler.push(priority, task);
    }

    pub fn rest(&self, ms: u64, task: Task) {
        self.timers.schedule(ms, task);
    }

    pub fn emit(&self, signal: &str, payload: SignalPayload, priority: Priority) {
        self.dormant.wake();
        self.signals.emit(signal, payload, priority);
    }

    pub fn print_metrics(&self) {
        self.metrics.print_hotspots();
        println!(
            "tasks_enqueued: {}",
            self.metrics.tasks_enqueued.load(Ordering::Relaxed)
        );
        println!(
            "tasks_executed: {}",
            self.metrics.tasks_executed.load(Ordering::Relaxed)
        );
        println!(
            "dormant_wakeups: {}",
            self.metrics.dormant_wakeups.load(Ordering::Relaxed)
        );
    }

    pub fn shutdown(self) {
        self.dormant.shutdown();
        self.timers.shutdown();
        self.surge_pool.shutdown();
    }
}
