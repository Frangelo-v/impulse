use std::collections::{HashMap, VecDeque};

// ── Delivery modes ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum DeliveryMode {
    Broadcast,
    Queue,
    Latest,
    Buffer(usize),
}

impl Default for DeliveryMode {
    fn default() -> Self { DeliveryMode::Broadcast }
}

// ── Activation record ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Activation<T> {
    pub id:      u64,
    pub signal:  String,
    pub handler: usize,
    pub payload: Vec<T>,
    pub depth:   u32,
}

// ── Stats ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct KernelStats {
    pub signals_emitted:       u64,
    pub activations_enqueued:  u64,
    pub activations_completed: u64,
    pub max_queue_depth:       usize,
}

// ── ActivationKernel ──────────────────────────────────────────────────────────

pub struct ActivationKernel<T> {
    next_id:  u64,
    handlers: HashMap<String, Vec<usize>>,
    modes:    HashMap<String, DeliveryMode>,

    // Per-signal state for delivery modes
    latest:   HashMap<String, Vec<T>>,           // Latest: only keep newest payload
    rings:    HashMap<String, VecDeque<Vec<T>>>, // Buffer: ring buffer of payloads

    queue: VecDeque<Activation<T>>,
    stats: KernelStats,
}

impl<T: Clone> ActivationKernel<T> {
    pub fn new() -> Self {
        Self {
            next_id:  1,
            handlers: HashMap::new(),
            modes:    HashMap::new(),
            latest:   HashMap::new(),
            rings:    HashMap::new(),
            queue:    VecDeque::new(),
            stats:    KernelStats::default(),
        }
    }

    pub fn register_handler(&mut self, signal: impl Into<String>, handler: usize) {
        self.handlers.entry(signal.into()).or_default().push(handler);
    }

    pub fn set_mode(&mut self, signal: impl Into<String>, mode: DeliveryMode) {
        self.modes.insert(signal.into(), mode);
    }

    pub fn emit(&mut self, signal: impl Into<String>, payload: Vec<T>, depth: u32) {
        let signal = signal.into();
        self.stats.signals_emitted += 1;

        let Some(handlers) = self.handlers.get(&signal) else { return };
        let handlers = handlers.clone();
        let mode = self.modes.get(&signal).cloned().unwrap_or_default();

        match mode {
            DeliveryMode::Broadcast => {
                self.enqueue_all(&signal, &handlers, payload, depth);
            }
            DeliveryMode::Queue => {
                // Only the first registered handler consumes each emission
                if let Some(&handler) = handlers.first() {
                    self.push_activation(signal, handler, payload, depth);
                }
            }
            DeliveryMode::Latest => {
                // Replace any pending payload; enqueue only if not already pending
                let already_pending = self.queue.iter().any(|a| a.signal == signal);
                self.latest.insert(signal.clone(), payload);
                if !already_pending {
                    if let Some(&handler) = handlers.first() {
                        let latest_payload = self.latest.get(&signal).cloned().unwrap_or_default();
                        self.push_activation(signal, handler, latest_payload, depth);
                    }
                }
            }
            DeliveryMode::Buffer(cap) => {
                let ring = self.rings.entry(signal.clone()).or_default();
                if ring.len() >= cap && cap > 0 {
                    ring.pop_front(); // drop oldest
                }
                ring.push_back(payload);
                // Drain ring into queue
                while let Some(p) = self.rings.get_mut(&signal).and_then(|r| r.pop_front()) {
                    self.enqueue_all(&signal, &handlers, p, depth);
                }
            }
        }

        self.stats.max_queue_depth = self.stats.max_queue_depth.max(self.queue.len());
    }

    fn enqueue_all(&mut self, signal: &str, handlers: &[usize], payload: Vec<T>, depth: u32) {
        for &handler in handlers {
            self.push_activation(signal.to_string(), handler, payload.clone(), depth);
        }
    }

    fn push_activation(&mut self, signal: String, handler: usize, payload: Vec<T>, depth: u32) {
        self.queue.push_back(Activation {
            id: self.next_id,
            signal,
            handler,
            payload,
            depth,
        });
        self.next_id += 1;
        self.stats.activations_enqueued += 1;
    }

    pub fn pop(&mut self) -> Option<Activation<T>> {
        self.queue.pop_front()
    }

    pub fn complete(&mut self) {
        self.stats.activations_completed += 1;
    }

    pub fn stats(&self) -> &KernelStats {
        &self.stats
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}
