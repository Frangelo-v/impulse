// Impulse Tree-Walk Evaluator — v0.1
//
// Pipeline for signal dispatch:
//   1. Signal emission  → ActivationKernel (Signal Registry + Scheduler)
//   2. Kernel drain     → Propagation Engine (runs reactive handlers)
//   3. Domain prefix    → Domain Manager (namespaced signal routing)
//   4. Surge flags      → Supervisor (lifecycle + shutdown)
//   5. surge_flags empty → Dormant Monitor (program may exit)
//
// Normal computation (nodes, loops, match, actors) stays synchronous
// and fast — it never touches the signal pipeline.

use crate::ast::*;
use crate::runtime::kernel::ActivationKernel;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::thread;
use std::time::Duration;

// ── Public surface ────────────────────────────────────────────────────────────

pub type RunResult<T> = Result<T, String>;

#[derive(Debug, Clone, Copy, Default)]
pub struct RunOptions {
    pub emit_stats: bool,
}

pub fn run_with_options(program: &Program, options: RunOptions) -> RunResult<()> {
    let eval = Evaluator::new(program)?;
    let result = eval.run(options);
    eval.shutdown();
    result
}

// ── Value ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Null,
    List(Vec<Value>),
    Map(Vec<(String, Value)>),
    Err { message: String, code: i64 },
    Object  { name: String, fields: HashMap<String, Value> },
    Variant { variant: String, case: String, fields: HashMap<String, Value> },
}

// Value is pure data — no Rc, no raw pointers.
unsafe impl Send for Value {}
unsafe impl Sync for Value {}

impl Value {
    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(v)   => *v,
            Value::Null      => false,
            Value::Int(v)    => *v != 0,
            Value::Float(v)  => *v != 0.0,
            Value::Str(v)    => !v.is_empty(),
            Value::List(v)   => !v.is_empty(),
            Value::Map(v)    => !v.is_empty(),
            Value::Err { .. } => false,
            _                => true,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_)        => "Int",
            Value::Float(_)      => "Float",
            Value::Bool(_)       => "Bool",
            Value::Str(_)        => "Str",
            Value::Null          => "Null",
            Value::List(_)       => "List",
            Value::Map(_)        => "Map",
            Value::Err { .. }    => "error",
            Value::Object { name, .. } if name == "error" => "error",
            Value::Object { .. } => "cluster",
            Value::Variant { .. } => "enum",
        }
    }

    pub fn display(&self) -> String {
        match self {
            Value::Int(v)    => v.to_string(),
            Value::Float(v)  => v.to_string(),
            Value::Bool(v)   => v.to_string(),
            Value::Str(v)    => v.clone(),
            Value::Null      => "null".into(),
            Value::Err { message, code } => format!("error({}: {})", code, message),
            Value::List(items) => {
                let inner = items.iter().map(|v| v.display()).collect::<Vec<_>>().join(", ");
                format!("[{}]", inner)
            }
            Value::Map(entries) => {
                let inner = entries.iter()
                    .map(|(k, v)| format!("{}: {}", k, v.display()))
                    .collect::<Vec<_>>().join(", ");
                format!("{{{}}}", inner)
            }
            Value::Object { name, fields } => {
                let body = fields.iter()
                    .map(|(k, v)| format!("{}: {}", k, v.display()))
                    .collect::<Vec<_>>().join(", ");
                format!("{} {{ {} }}", name, body)
            }
            Value::Variant { variant, case, fields } => {
                if fields.is_empty() {
                    format!("{}.{}", variant, case)
                } else {
                    let body = fields.iter()
                        .map(|(k, v)| format!("{}: {}", k, v.display()))
                        .collect::<Vec<_>>().join(", ");
                    format!("{}.{} {{ {} }}", variant, case, body)
                }
            }
        }
    }

    fn eq_val(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a),   Value::Int(b))   => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Bool(a),  Value::Bool(b))  => a == b,
            (Value::Str(a),   Value::Str(b))   => a == b,
            (Value::Null,     Value::Null)     => true,
            _ => self.display() == other.display(),
        }
    }
}

// ── Control flow ──────────────────────────────────────────────────────────────

enum Flow {
    Continue(Value),
    Pulse(Value),
    Break,
}

// ── Lexical environment ───────────────────────────────────────────────────────

#[derive(Default, Clone)]
struct Env {
    scopes:     Vec<HashMap<String, Value>>,
    self_value: Option<Value>,
}

impl Env {
    fn new() -> Self {
        Self { scopes: vec![HashMap::new()], self_value: None }
    }

    fn child(&self) -> Self {
        let mut next = self.clone();
        next.scopes.push(HashMap::new());
        next
    }

    fn with_self(&self, sv: Value) -> Self {
        let mut next = self.child();
        next.self_value = Some(sv);
        next
    }

    fn set_local(&mut self, name: impl Into<String>, value: Value) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.into(), value);
        }
    }

    fn assign(&mut self, name: &str, value: Value) -> bool {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), value);
                return true;
            }
        }
        false
    }

    fn get(&self, name: &str) -> Option<Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) { return Some(v.clone()); }
        }
        None
    }
}

// ── Owned program tables ──────────────────────────────────────────────────────

struct ProgramTables {
    nodes:            HashMap<String, NodeDecl>,
    clusters:         HashMap<String, ClusterDecl>,
    actors:           HashMap<String, ActorDecl>,
    variants:         HashMap<String, VariantDecl>,
    active_surges:    HashMap<String, SurgeDecl>,
    supervisors:      HashMap<String, SupervisorDecl>,
    reactive_handlers: Vec<(String, SurgeDecl)>,
}

impl ProgramTables {
    fn build(program: &Program) -> Self {
        let mut t = ProgramTables {
            nodes: HashMap::new(),
            clusters: HashMap::new(),
            actors: HashMap::new(),
            variants: HashMap::new(),
            active_surges: HashMap::new(),
            supervisors: HashMap::new(),
            reactive_handlers: Vec::new(),
        };
        for decl in &program.decls {
            t.collect_decl(decl, None);
        }
        t
    }

    fn collect_decl(&mut self, decl: &TopDecl, domain: Option<&str>) {
        match decl {
            TopDecl::Node(n)       => { self.nodes.insert(n.name.clone(), n.clone()); }
            TopDecl::Cluster(c)    => { self.clusters.insert(c.name.clone(), c.clone()); }
            TopDecl::Actor(a)      => { self.actors.insert(a.name.clone(), a.clone()); }
            TopDecl::Variant(v)    => { self.variants.insert(v.name.clone(), v.clone()); }
            TopDecl::Supervisor(s) => { self.supervisors.insert(s.name.clone(), s.clone()); }
            TopDecl::Surge(s) => match &s.kind {
                SurgeKind::Active { name } => { self.active_surges.insert(name.clone(), s.clone()); }
                SurgeKind::Reactive { signal } => {
                    let qualified = domain
                        .map(|d| format!("{}::{}", d, signal.name))
                        .unwrap_or_else(|| signal.qualified());
                    self.reactive_handlers.push((qualified, s.clone()));
                }
            },
            TopDecl::Domain(d) => {
                for surge in &d.surges {
                    self.collect_decl(&TopDecl::Surge(surge.clone()), Some(&d.name));
                }
            }
            _ => {}
        }
    }
}

// ── Shared brain state ────────────────────────────────────────────────────────

struct BrainState {
    // Scalar module state (cortex)
    shared:         Mutex<HashMap<String, Value>>,
    // Serialized actor fields
    actor_state:    Mutex<HashMap<String, HashMap<String, Value>>>,
    // Signal Registry + Scheduler — the activation queue
    kernel:         Mutex<ActivationKernel<Value>>,
    // Trace channels (typed backpressure)
    trace_channels: Mutex<HashMap<String, std::collections::VecDeque<Value>>>,
    // Despierta a los `await` bloqueados cuando llega un valor — cero polling
    trace_ready:    Condvar,
}

impl BrainState {
    fn new(program: &Program, tables: &ProgramTables) -> Self {
        let mut shared = HashMap::new();
        let mut actor_state: HashMap<String, HashMap<String, Value>> = HashMap::new();

        for decl in &program.decls {
            if let TopDecl::Cortex(c) = decl {
                if let Ok(v) = eval_const(& c.value) { shared.insert(c.name.clone(), v); }
            }
        }
        for (name, actor) in &tables.actors {
            let mut fields = HashMap::new();
            for f in &actor.fields {
                if let Ok(v) = eval_const(&f.value) { fields.insert(f.name.clone(), v); }
            }
            actor_state.insert(name.clone(), fields);
        }

        let mut kernel = ActivationKernel::new();
        for (i, (signal_name, _)) in tables.reactive_handlers.iter().enumerate() {
            kernel.register_handler(signal_name.clone(), i);
        }

        BrainState {
            shared:         Mutex::new(shared),
            actor_state:    Mutex::new(actor_state),
            kernel:         Mutex::new(kernel),
            trace_channels: Mutex::new(HashMap::new()),
            trace_ready:    Condvar::new(),
        }
    }
}

// ── Surge management ──────────────────────────────────────────────────────────

struct SurgeEntry {
    shutdown: Arc<AtomicBool>,
    handle:   thread::JoinHandle<()>,
}

// ── Evaluator ─────────────────────────────────────────────────────────────────

pub struct Evaluator {
    tables: Arc<ProgramTables>,
    state:  Arc<BrainState>,
    surges: Mutex<HashMap<String, SurgeEntry>>,
    this:   OnceLock<Weak<Self>>,
}

// Per-thread break-pending flag — each thread (main, surge) has its own copy.
// Set by `break`, cleared by the Loop that catches it.
thread_local! {
    static BREAK_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn set_break()      { BREAK_PENDING.with(|b| b.set(true)); }
fn clear_break()    { BREAK_PENDING.with(|b| b.set(false)); }
fn is_break() -> bool { BREAK_PENDING.with(|b| b.get()) }

// Per-thread pending pulse value — set by `pulse`, consumed at the node
// boundary (call_node / actor method / cluster method / handler / main).
// Lets `pulse` inside nested ifs, loops, and match arms exit the whole node,
// since eval_expr collapses Flow at those boundaries.
thread_local! {
    static PULSE_PENDING: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
}

fn set_pulse(v: Value)          { PULSE_PENDING.with(|p| *p.borrow_mut() = Some(v)); }
fn take_pulse() -> Option<Value> { PULSE_PENDING.with(|p| p.borrow_mut().take()) }
fn pulse_value() -> Option<Value> { PULSE_PENDING.with(|p| p.borrow().clone()) }

impl Evaluator {
    pub fn new(program: &Program) -> RunResult<Arc<Self>> {
        let tables = Arc::new(ProgramTables::build(program));
        let state  = Arc::new(BrainState::new(program, &tables));
        let eval   = Arc::new(Evaluator {
            tables,
            state,
            surges: Mutex::new(HashMap::new()),
            this: OnceLock::new(),
        });
        eval.this.set(Arc::downgrade(&eval)).ok();
        Ok(eval)
    }

    fn me(&self) -> Arc<Self> {
        self.this.get().unwrap().upgrade().unwrap()
    }

    // ── Entry point ───────────────────────────────────────────────────────────

    pub fn run(&self, options: RunOptions) -> RunResult<()> {
        let main = self.tables.nodes.get("main")
            .ok_or_else(|| "missing `node main()` — every Impulse program needs an entry point".to_string())?
            .clone();
        let mut env = Env::new();
        let main_flow = self.exec_block(&main.body, &mut env);
        take_pulse();
        main_flow?;
        // Propagation Engine: drain all pending reactive activations
        self.drain_propagation()?;

        if options.emit_stats {
            let k = self.state.kernel.lock().unwrap();
            let s = k.stats();
            println!("=== Runtime Stats ===");
            println!("signals_emitted:       {}", s.signals_emitted);
            println!("activations_enqueued:  {}", s.activations_enqueued);
            println!("activations_completed: {}", s.activations_completed);
            println!("max_queue_depth:       {}", s.max_queue_depth);
        }
        Ok(())
    }

    // Signal all active surges to stop, then join their threads.
    pub fn shutdown(&self) {
        let mut surges = self.surges.lock().unwrap();
        for (_, entry) in surges.iter() {
            entry.shutdown.store(true, Ordering::Release);
        }
        for (_, entry) in surges.drain() {
            let _ = entry.handle.join();
        }
    }

    // ── Propagation Engine ────────────────────────────────────────────────────
    // Drains the kernel queue, executing each reactive handler synchronously.

    fn drain_propagation(&self) -> RunResult<()> {
        loop {
            let activation = self.state.kernel.lock().unwrap().pop();
            let Some(act) = activation else { break };

            let Some((_, surge)) = self.tables.reactive_handlers.get(act.handler).cloned() else {
                self.state.kernel.lock().unwrap().complete();
                continue;
            };
            let mut env = Env::new();
            for (p, v) in surge.params.iter().zip(act.payload.iter()) {
                env.set_local(p.name.clone(), v.clone());
            }
            let handler_flow = self.exec_block(&surge.body, &mut env);
            take_pulse();
            handler_flow?;
            self.state.kernel.lock().unwrap().complete();
            // Signals emitted inside a handler are queued and drained here
        }
        Ok(())
    }

    // ── Statement execution ───────────────────────────────────────────────────

    fn exec_block(&self, body: &[Stmt], env: &mut Env) -> RunResult<Flow> {
        let mut last = Value::Null;
        for stmt in body {
            if is_break() { return Ok(Flow::Break); }
            match self.exec_stmt(stmt, env)? {
                Flow::Continue(v) => last = v,
                other             => return Ok(other),
            }
            // Check AFTER each statement — break/pulse may have been set inside an `if` body
            if is_break() { return Ok(Flow::Break); }
            if let Some(v) = pulse_value() { return Ok(Flow::Pulse(v)); }
        }
        Ok(Flow::Continue(last))
    }

    fn exec_stmt(&self, stmt: &Stmt, env: &mut Env) -> RunResult<Flow> {
        match stmt {
            Stmt::Decl(TopDecl::Charge(decl)) => {
                let v = self.eval_expr(&decl.value, env)?;
                env.set_local(decl.name.clone(), v);
                Ok(Flow::Continue(Value::Null))
            }
            Stmt::Decl(_) => Ok(Flow::Continue(Value::Null)),
            Stmt::Expr(expr) => {
                if let ExprKind::Pulse(inner) = &expr.kind {
                    let v = self.eval_expr(inner, env)?;
                    set_pulse(v.clone());
                    return Ok(Flow::Pulse(v));
                }
                Ok(Flow::Continue(self.eval_expr(expr, env)?))
            }
            Stmt::Break(_) => {
                set_break();
                Ok(Flow::Break)
            }
            Stmt::TraceSend { channel, value, .. } => {
                let name = self.eval_expr(channel, env)?.display();
                let val  = self.eval_expr(value, env)?;
                self.state.trace_channels.lock().unwrap()
                    .entry(name).or_default().push_back(val);
                self.state.trace_ready.notify_all();
                Ok(Flow::Continue(Value::Null))
            }
        }
    }

    // ── Expression evaluation ─────────────────────────────────────────────────

    fn eval_expr(&self, expr: &Expr, env: &mut Env) -> RunResult<Value> {
        match &expr.kind {

            // ── Literals ─────────────────────────────────────────────────────
            ExprKind::Int(v)   => Ok(Value::Int(*v)),
            ExprKind::Float(v) => Ok(Value::Float(*v)),
            ExprKind::Bool(v)  => Ok(Value::Bool(*v)),
            ExprKind::Str(v)   => Ok(Value::Str(v.clone())),
            ExprKind::Null     => Ok(Value::Null),

            // ── References ───────────────────────────────────────────────────
            ExprKind::Ident(name) => {
                if name == "shared" {
                    let fields = self.state.shared.lock().unwrap().clone();
                    return Ok(Value::Object { name: "shared".into(), fields });
                }
                Ok(env.get(name).unwrap_or(Value::Null))
            }
            ExprKind::SelfKw => env.self_value.clone()
                .ok_or_else(|| "`self` used outside a method".to_string()),
            ExprKind::DomainRef { domain, name } =>
                Ok(env.get(&format!("{}::{}", domain, name)).unwrap_or(Value::Null)),

            // ── Collections ──────────────────────────────────────────────────
            ExprKind::ListLit(exprs) => {
                let items = exprs.iter().map(|e| self.eval_expr(e, env)).collect::<RunResult<_>>()?;
                Ok(Value::List(items))
            }
            ExprKind::MapLit(pairs) => {
                let mut map = Vec::new();
                for (k, v) in pairs {
                    map.push((self.eval_expr(k, env)?.display(), self.eval_expr(v, env)?));
                }
                Ok(Value::Map(map))
            }
            ExprKind::TupleLit(exprs) => {
                let items = exprs.iter().map(|e| self.eval_expr(e, env)).collect::<RunResult<_>>()?;
                Ok(Value::List(items))
            }
            ExprKind::Index { object, index } => {
                let obj = self.eval_expr(object, env)?;
                let idx = self.eval_expr(index, env)?;
                Ok(match (obj, idx) {
                    (Value::List(v), Value::Int(i)) if i >= 0 =>
                        v.get(i as usize).cloned().unwrap_or(Value::Null),
                    (Value::Map(m), Value::Str(k)) =>
                        m.into_iter().find(|(key, _)| key == &k).map(|(_, v)| v).unwrap_or(Value::Null),
                    _ => Value::Null,
                })
            }

            // ── Constructors ─────────────────────────────────────────────────
            ExprKind::ClusterLit { name, fields } => {
                let mut values = HashMap::new();
                for (f, e) in fields { values.insert(f.clone(), self.eval_expr(e, env)?); }
                Ok(Value::Object { name: name.clone(), fields: values })
            }
            ExprKind::VariantLit { variant, case, payload } => {
                let mut fields = HashMap::new();
                match payload {
                    VariantPayload::Struct(entries) =>
                        for (f, e) in entries { fields.insert(f.clone(), self.eval_expr(e, env)?); },
                    VariantPayload::Tuple(exprs) =>
                        for (i, e) in exprs.iter().enumerate() {
                            fields.insert(format!("_{}", i), self.eval_expr(e, env)?);
                        },
                    VariantPayload::None => {}
                }
                Ok(Value::Variant { variant: variant.clone(), case: case.clone(), fields })
            }

            // ── Field access ─────────────────────────────────────────────────
            ExprKind::Field { object, name } => {
                // `shared.x` lee solo esa clave — sin clonar todo el estado
                if matches!(&object.kind, ExprKind::Ident(n) if n == "shared") {
                    return Ok(self.state.shared.lock().unwrap()
                        .get(name).cloned().unwrap_or(Value::Null));
                }
                // Unit variant reference: `Status.Online` (no payload braces)
                if let ExprKind::Ident(type_name) = &object.kind {
                    if let Some(decl) = self.tables.variants.get(type_name) {
                        if decl.cases.iter().any(|c| &c.name == name) {
                            return Ok(Value::Variant {
                                variant: type_name.clone(),
                                case: name.clone(),
                                fields: HashMap::new(),
                            });
                        }
                    }
                }
                let obj = self.eval_expr(object, env)?;
                Ok(self.get_field(&obj, name))
            }

            // ── Operators ────────────────────────────────────────────────────
            ExprKind::Binary { op, left, right } => {
                let l = self.eval_expr(left, env)?;
                let r = self.eval_expr(right, env)?;
                self.eval_binary(*op, l, r)
            }
            ExprKind::Unary { op, operand } => {
                let v = self.eval_expr(operand, env)?;
                Ok(match (op, v) {
                    (UnOp::Neg, Value::Int(n))   => Value::Int(-n),
                    (UnOp::Neg, Value::Float(f)) => Value::Float(-f),
                    (UnOp::Not, v)               => Value::Bool(!v.truthy()),
                    _                            => Value::Null,
                })
            }
            ExprKind::Assign { target, op, value } => {
                let rhs = self.eval_expr(value, env)?;
                self.assign_target(target, *op, rhs, env)
            }

            // ── Calls ────────────────────────────────────────────────────────
            ExprKind::Call { callee, args } => match &callee.kind {
                ExprKind::Ident(name) => {
                    let arg_vals = self.eval_args(args, env)?;
                    self.call_node(name, arg_vals)
                }
                ExprKind::Field { object, name } =>
                    self.eval_field_call(object, name, args, env),
                ExprKind::DomainRef { name, .. } => {
                    let arg_vals = self.eval_args(args, env)?;
                    self.call_node(name, arg_vals)
                }
                _ => Err(format!("unsupported call: {:?}", callee.kind)),
            },
            ExprKind::Method { object, name, args } =>
                self.eval_field_call(object, name, args, env),

            // ── Pipe ─────────────────────────────────────────────────────────
            ExprKind::Pipe(expr, node_name) => {
                let val = self.eval_expr(expr, env)?;
                self.call_node(node_name, vec![val])
            }

            // ── Error handling ────────────────────────────────────────────────
            ExprKind::Try(expr) => {
                // If result is an error, return it upward (early pulse)
                self.eval_expr(expr, env)
            }
            ExprKind::Coalesce(a, b) => {
                let val = self.eval_expr(a, env)?;
                match val {
                    Value::Null | Value::Err { .. } => self.eval_expr(b, env),
                    other => Ok(other),
                }
            }

            // ── Node return ──────────────────────────────────────────────────
            ExprKind::Pulse(expr) | ExprKind::Yield(expr) | ExprKind::Collect(expr) =>
                self.eval_expr(expr, env),

            // ── Signal emission (Signal Registry → Scheduler) ────────────────
            ExprKind::Signal { name, args, priority, .. } => {
                let arg_vals = self.eval_args(args, env)?;
                let prio = match priority {
                    SignalPriority::Idle     => 0u32,
                    SignalPriority::Normal   => 1,
                    SignalPriority::Critical => 2,
                    SignalPriority::Realtime => 3,
                };
                self.state.kernel.lock().unwrap()
                    .emit(name.qualified(), arg_vals, prio);
                // Immediately drain so reactive handlers run in order
                self.drain_propagation()?;
                Ok(Value::Null)
            }

            // ── Surge lifecycle ───────────────────────────────────────────────
            ExprKind::Open { label, callee, args } =>
                self.start_surge(label.as_deref(), callee, args, env),

            ExprKind::OpenSupervisor(name) =>
                self.start_supervisor(name),

            ExprKind::Close(label) => {
                if let Some(entry) = self.surges.lock().unwrap().get(label) {
                    entry.shutdown.store(true, Ordering::Release);
                }
                Ok(Value::Null)
            }

            // sleep — parks the current thread (no polling)
            ExprKind::Rest(ms_expr) => {
                let ms = match self.eval_expr(ms_expr, env)? {
                    Value::Int(n) => n as u64,
                    _ => 0,
                };
                thread::sleep(Duration::from_millis(ms));
                Ok(Value::Null)
            }

            // ── Trace channels ────────────────────────────────────────────────
            // El hilo se aparca en un Condvar hasta que TraceSend notifique —
            // garantía nº 6 de la spec: nada de sleep con polling.
            ExprKind::TraceRecv(ch_expr) => {
                let name = self.eval_expr(ch_expr, env)?.display();
                let mut chans = self.state.trace_channels.lock().unwrap();
                loop {
                    if let Some(v) = chans.get_mut(&name).and_then(|q| q.pop_front()) {
                        return Ok(v);
                    }
                    chans = self.state.trace_ready.wait(chans).unwrap();
                }
            }
            ExprKind::Select { arms } => {
                // Los nombres se evalúan una vez, fuera del lock, para no
                // ejecutar código de usuario con el mutex de canales tomado.
                let names: Vec<String> = arms.iter()
                    .map(|a| self.eval_expr(&a.channel, env).map(|v| v.display()))
                    .collect::<RunResult<_>>()?;
                let mut chans = self.state.trace_channels.lock().unwrap();
                loop {
                    for (arm, name) in arms.iter().zip(&names) {
                        if let Some(v) = chans.get_mut(name).and_then(|q| q.pop_front()) {
                            drop(chans);
                            let mut child = env.child();
                            child.set_local(arm.binding.clone(), v);
                            return match self.exec_block(&arm.body, &mut child)? {
                                Flow::Pulse(v) | Flow::Continue(v) => Ok(v),
                                Flow::Break => Ok(Value::Null),
                            };
                        }
                    }
                    chans = self.state.trace_ready.wait(chans).unwrap();
                }
            }

            // ── Control flow ──────────────────────────────────────────────────
            ExprKind::When { cond, then, else_ifs, else_ } => {
                if self.eval_expr(cond, env)?.truthy() {
                    return self.exec_if_branch(then, env);
                }
                for (ec, eb) in else_ifs {
                    if self.eval_expr(ec, env)?.truthy() {
                        return self.exec_if_branch(eb, env);
                    }
                }
                if let Some(b) = else_ { self.exec_if_branch(b, env) }
                else { Ok(Value::Null) }
            }

            ExprKind::Loop { binding: None, body } => {
                // Run directly in env so mutations to outer variables persist
                loop {
                    match self.exec_block(body, env)? {
                        Flow::Break => { clear_break(); break; }
                        Flow::Continue(_) => {}
                        Flow::Pulse(v)    => return Ok(v),
                    }
                }
                Ok(Value::Null)
            }
            ExprKind::Loop { binding: Some((name, iter_expr)), body } => {
                let items = match self.eval_expr(iter_expr, env)? {
                    Value::List(v) => v,
                    single         => vec![single],
                };
                for item in items {
                    // Bind the loop variable in the outer env so mutations persist
                    env.set_local(name.clone(), item);
                    match self.exec_block(body, env)? {
                        Flow::Break => { clear_break(); break; }
                        Flow::Continue(_) => {}
                        Flow::Pulse(v)    => return Ok(v),
                    }
                }
                Ok(Value::Null)
            }

            ExprKind::Match { subject, arms } => {
                let val = self.eval_expr(subject, env)?;
                for arm in arms {
                    let mut arm_env = env.child();
                    if self.pattern_matches(&arm.pattern, &val, &mut arm_env) {
                        return match self.exec_block(&arm.body, &mut arm_env)? {
                            Flow::Pulse(v) | Flow::Continue(v) => Ok(v),
                            Flow::Break => Ok(Value::Null),
                        };
                    }
                }
                Ok(Value::Null)
            }

            ExprKind::Break => {
                set_break();
                Ok(Value::Null)
            }
        }
    }

    // ── If/else helper ────────────────────────────────────────────────────────

    fn exec_if_branch(&self, body: &[Stmt], env: &mut Env) -> RunResult<Value> {
        match self.exec_block(body, &mut env.child())? {
            Flow::Pulse(v) | Flow::Continue(v) => Ok(v),
            Flow::Break => Ok(Value::Null),
        }
    }

    // ── Surge spawning ────────────────────────────────────────────────────────

    fn start_surge(
        &self,
        label: Option<&str>,
        callee: &Expr,
        args: &[Expr],
        env: &mut Env,
    ) -> RunResult<Value> {
        let name = match &callee.kind {
            ExprKind::Ident(n) => n.clone(),
            _ => return Err("start requires a surge name".into()),
        };
        let surge = self.tables.active_surges.get(&name)
            .ok_or_else(|| format!("unknown surge '{}'", name))?.clone();
        let arg_vals  = self.eval_args(args, env)?;
        let label     = label.unwrap_or(&name).to_string();
        let shutdown  = Arc::new(AtomicBool::new(false));
        let eval      = self.me();
        let lbl2      = label.clone();
        let sd2       = Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            let mut env = Env::new();
            for (p, v) in surge.params.iter().zip(arg_vals.into_iter()) {
                env.set_local(p.name.clone(), v);
            }
            eval.exec_surge_body(&surge.body, &mut env, &sd2);
            // Surge exited — remove itself from the registry
            eval.surges.lock().unwrap().remove(&lbl2);
        });

        self.surges.lock().unwrap().insert(label.clone(), SurgeEntry { shutdown, handle });
        Ok(Value::Str(label))
    }

    fn exec_surge_body(&self, body: &[Stmt], env: &mut Env, shutdown: &Arc<AtomicBool>) {
        for stmt in body {
            if shutdown.load(Ordering::Acquire) { break; }
            match stmt {
                Stmt::Expr(expr) => {
                    match &expr.kind {
                        // Loop inside a surge respects the shutdown flag
                        ExprKind::Loop { binding: None, body } => {
                            loop {
                                if shutdown.load(Ordering::Acquire) { return; }
                                match self.exec_block(body, env) {
                                    Ok(Flow::Break)    => { clear_break(); break; }
                                    Ok(Flow::Continue(_)) => {}
                                    Ok(Flow::Pulse(_)) | Err(_) => return,
                                }
                            }
                        }
                        ExprKind::Loop { binding: Some((name, iter_expr)), body } => {
                            let items = match self.eval_expr(iter_expr, env) {
                                Ok(Value::List(v)) => v,
                                Ok(v)              => vec![v],
                                Err(_)             => return,
                            };
                            for item in items {
                                if shutdown.load(Ordering::Acquire) { return; }
                                env.set_local(name.clone(), item);
                                match self.exec_block(body, env) {
                                    Ok(Flow::Break)    => { clear_break(); break; }
                                    Ok(Flow::Continue(_)) => {}
                                    Ok(Flow::Pulse(_)) | Err(_) => return,
                                }
                            }
                        }
                        _ => { self.exec_stmt(stmt, env).ok(); }
                    }
                }
                _ => { self.exec_stmt(stmt, env).ok(); }
            }
        }
    }

    fn start_supervisor(&self, name: &str) -> RunResult<Value> {
        let sup = self.tables.supervisors.get(name)
            .ok_or_else(|| format!("unknown supervisor '{}'", name))?.clone();

        for child in &sup.children {
            let surge = self.tables.active_surges.get(&child.callee)
                .ok_or_else(|| format!("supervisor child '{}' calls unknown surge '{}'", child.label, child.callee))?
                .clone();

            let label    = child.label.clone();
            let shutdown = Arc::new(AtomicBool::new(false));
            let eval     = self.me();
            let lbl2     = label.clone();
            let sd2      = Arc::clone(&shutdown);

            let handle = thread::spawn(move || {
                let mut env = Env::new();
                eval.exec_surge_body(&surge.body, &mut env, &sd2);
                eval.surges.lock().unwrap().remove(&lbl2);
            });

            self.surges.lock().unwrap().insert(label, SurgeEntry { shutdown, handle });
        }
        Ok(Value::Null)
    }

    // ── Node / method dispatch ────────────────────────────────────────────────

    fn call_node(&self, name: &str, args: Vec<Value>) -> RunResult<Value> {
        let node = self.tables.nodes.get(name)
            .ok_or_else(|| format!("unknown node '{}'", name))?.clone();
        let mut env = Env::new();
        for (p, v) in node.params.iter().zip(args) { env.set_local(p.name.clone(), v); }
        let flow = self.exec_block(&node.body, &mut env);
        take_pulse();
        Ok(match flow? {
            Flow::Pulse(v) | Flow::Continue(v) => v,
            Flow::Break => Value::Null,
        })
    }

    fn call_actor_method(&self, actor_name: &str, method_name: &str, args: Vec<Value>) -> RunResult<Value> {
        let actor  = self.tables.actors.get(actor_name)
            .ok_or_else(|| format!("unknown actor '{}'", actor_name))?.clone();
        let method = actor.methods.iter().find(|m| m.name == method_name)
            .ok_or_else(|| format!("unknown method '{}.{}'", actor_name, method_name))?.clone();

        let snap = self.state.actor_state.lock().unwrap().get(actor_name).cloned().unwrap_or_default();
        let mut env = Env::new().with_self(Value::Object { name: actor_name.to_string(), fields: snap });
        for (p, v) in method.params.iter().zip(args) { env.set_local(p.name.clone(), v); }
        let out = self.exec_block(&method.body, &mut env);
        take_pulse();
        let out = out?;

        if let Some(Value::Object { fields, .. }) = env.self_value {
            self.state.actor_state.lock().unwrap().insert(actor_name.to_string(), fields);
        }
        Ok(match out { Flow::Pulse(v) | Flow::Continue(v) => v, Flow::Break => Value::Null })
    }

    fn call_cluster_method(&self, obj: Value, method: &str, args: Vec<Value>) -> RunResult<Value> {
        let Value::Object { name: cluster_name, fields } = obj else {
            return Err("cannot call method on non-cluster value".into());
        };
        let cluster = self.tables.clusters.get(&cluster_name)
            .ok_or_else(|| format!("unknown cluster '{}'", cluster_name))?.clone();
        let method_decl = cluster.methods.iter().find(|m| m.name == method)
            .ok_or_else(|| format!("unknown method '{}.{}'", cluster_name, method))?.clone();
        let mut env = Env::new().with_self(Value::Object { name: cluster_name, fields });
        for (p, v) in method_decl.params.iter().zip(args) { env.set_local(p.name.clone(), v); }
        let flow = self.exec_block(&method_decl.body, &mut env);
        take_pulse();
        Ok(match flow? {
            Flow::Pulse(v) | Flow::Continue(v) => v,
            Flow::Break => Value::Null,
        })
    }

    fn eval_field_call(&self, object: &Expr, method: &str, args: &[Expr], env: &mut Env) -> RunResult<Value> {
        // io.* namespace
        if matches!(&object.kind, ExprKind::Ident(n) if n == "io") {
            return self.call_io(method, args, env);
        }
        // math.* namespace
        if matches!(&object.kind, ExprKind::Ident(n) if n == "math") {
            return self.call_math(method, args, env);
        }
        // time.* namespace
        if matches!(&object.kind, ExprKind::Ident(n) if n == "time") {
            return self.call_time(method, args, env);
        }
        // fs.* namespace
        if matches!(&object.kind, ExprKind::Ident(n) if n == "fs") {
            return self.call_fs(method, args, env);
        }
        // http.* namespace
        if matches!(&object.kind, ExprKind::Ident(n) if n == "http") {
            return self.call_http(method, args, env);
        }
        // Actor method
        if let ExprKind::Ident(actor_name) = &object.kind {
            if self.tables.actors.contains_key(actor_name) {
                let arg_vals = self.eval_args(args, env)?;
                return self.call_actor_method(actor_name, method, arg_vals);
            }
        }
        let obj_val  = self.eval_expr(object, env)?;
        let arg_vals = self.eval_args(args, env)?;
        match obj_val {
            Value::Str(_)    => self.call_str_method(obj_val, method, arg_vals),
            Value::List(_) | Value::Map(_) => {
                let mutating = match obj_val {
                    Value::List(_) => matches!(method, "push" | "pop"),
                    _              => matches!(method, "set" | "delete"),
                };
                let mut coll = obj_val;
                let result = match coll {
                    Value::List(_) => self.call_list_method(&mut coll, method, arg_vals)?,
                    _              => self.call_map_method(&mut coll, method, arg_vals)?,
                };
                // Los métodos que mutan escriben la colección de vuelta en la
                // variable; sobre un temporal serían un no-op silencioso — error.
                if mutating {
                    match &object.kind {
                        ExprKind::Ident(name) if env.assign(name, coll) => {}
                        ExprKind::Ident(name) => return Err(format!(
                            "cannot mutate '{}' — it is not a local variable", name
                        )),
                        _ => return Err(format!(
                            "'{}' mutates the collection — call it on a variable, not on a temporary value",
                            method
                        )),
                    }
                }
                Ok(result)
            }
            Value::Object { .. } => self.call_cluster_method(obj_val, method, arg_vals),
            _ => Err(format!("no method '{}' on {}", method, obj_val.type_name())),
        }
    }

    // ── Builtins ──────────────────────────────────────────────────────────────

    fn call_io(&self, method: &str, args: &[Expr], env: &mut Env) -> RunResult<Value> {
        let vals = self.eval_args(args, env)?;
        let v = vals.first().unwrap_or(&Value::Null).display();
        match method {
            "print" | "println" => { println!("{}", v); Ok(Value::Null) }
            "eprint"            => { eprintln!("{}", v); Ok(Value::Null) }
            _ => Err(format!("unknown io.{}", method)),
        }
    }

    // Errores de stdlib: mismo shape que `error { message, code }` del usuario,
    // así se matchean con `e: error` igual que cualquier otro.
    fn std_error(message: String, code: i64) -> Value {
        let mut fields = HashMap::new();
        fields.insert("message".to_string(), Value::Str(message));
        fields.insert("code".to_string(), Value::Int(code));
        Value::Object { name: "error".to_string(), fields }
    }

    fn call_time(&self, method: &str, args: &[Expr], env: &mut Env) -> RunResult<Value> {
        let _ = self.eval_args(args, env)?;
        match method {
            // Milisegundos desde epoch — suficiente para timestamps y medir duraciones
            "now" => {
                let ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                Ok(Value::Int(ms))
            }
            _ => Err(format!("unknown time.{} — available: now()", method)),
        }
    }

    fn call_fs(&self, method: &str, args: &[Expr], env: &mut Env) -> RunResult<Value> {
        let vals = self.eval_args(args, env)?;
        let path = vals.first().map(|v| v.display()).unwrap_or_default();
        match method {
            "read" => Ok(match std::fs::read_to_string(&path) {
                Ok(s)  => Value::Str(s),
                Err(e) => Self::std_error(format!("cannot read '{}': {}", path, e), 404),
            }),
            "write" | "append" => {
                let content = vals.get(1).map(|v| v.display()).unwrap_or_default();
                let result = if method == "write" {
                    std::fs::write(&path, &content)
                } else {
                    use std::io::Write;
                    std::fs::OpenOptions::new().create(true).append(true).open(&path)
                        .and_then(|mut f| f.write_all(content.as_bytes()))
                };
                Ok(match result {
                    Ok(())  => Value::Bool(true),
                    Err(e) => Self::std_error(format!("cannot {} '{}': {}", method, path, e), 500),
                })
            }
            "exists" => Ok(Value::Bool(std::path::Path::new(&path).exists())),
            "delete" => Ok(match std::fs::remove_file(&path) {
                Ok(())  => Value::Bool(true),
                Err(e) => Self::std_error(format!("cannot delete '{}': {}", path, e), 500),
            }),
            _ => Err(format!(
                "unknown fs.{} — available: read(path), write(path, content), append(path, content), exists(path), delete(path)",
                method
            )),
        }
    }

    fn call_http(&self, method: &str, args: &[Expr], env: &mut Env) -> RunResult<Value> {
        let vals = self.eval_args(args, env)?;
        let url = vals.first().map(|v| v.display()).unwrap_or_default();
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(30))
            .build();
        let response = match method {
            "get" => agent.get(&url).call(),
            "post" => {
                let body = vals.get(1).map(|v| v.display()).unwrap_or_default();
                agent.post(&url)
                    .set("Content-Type", "application/json")
                    .send_string(&body)
            }
            _ => return Err(format!("unknown http.{} — available: get(url), post(url, body)", method)),
        };
        Ok(match response {
            Ok(resp) => match resp.into_string() {
                Ok(body) => Value::Str(body),
                Err(e)   => Self::std_error(format!("cannot read response body: {}", e), 500),
            },
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                Self::std_error(format!("HTTP {}: {}", code, body), code as i64)
            }
            Err(e) => Self::std_error(format!("request failed: {}", e), 0),
        })
    }

    fn call_math(&self, method: &str, args: &[Expr], env: &mut Env) -> RunResult<Value> {
        let vals = self.eval_args(args, env)?;
        let f = |v: &Value| -> f64 { match v { Value::Int(n) => *n as f64, Value::Float(f) => *f, _ => 0.0 } };
        let a = vals.get(0).unwrap_or(&Value::Null);
        let b = vals.get(1).unwrap_or(&Value::Null);
        Ok(match method {
            "floor" => Value::Int(f(a).floor() as i64),
            "ceil"  => Value::Int(f(a).ceil() as i64),
            "sqrt"  => Value::Float(f(a).sqrt()),
            "abs"   => match a { Value::Int(n) => Value::Int(n.abs()), Value::Float(v) => Value::Float(v.abs()), _ => Value::Null },
            "min"   => Value::Float(f(a).min(f(b))),
            "max"   => Value::Float(f(a).max(f(b))),
            "pow"   => Value::Float(f(a).powf(f(b))),
            _ => return Err(format!("unknown math.{}", method)),
        })
    }

    fn call_str_method(&self, obj: Value, method: &str, args: Vec<Value>) -> RunResult<Value> {
        let Value::Str(s) = obj else { unreachable!() };
        let arg0 = || args.first().map(|v| v.display()).unwrap_or_default();
        Ok(match method {
            "len"         => Value::Int(s.chars().count() as i64),
            "contains"    => Value::Bool(s.contains(arg0().as_str())),
            "starts_with" => Value::Bool(s.starts_with(arg0().as_str())),
            "ends_with"   => Value::Bool(s.ends_with(arg0().as_str())),
            "to_upper"    => Value::Str(s.to_uppercase()),
            "to_lower"    => Value::Str(s.to_lowercase()),
            "trim"        => Value::Str(s.trim().to_string()),
            "split" => Value::List(s.split(arg0().as_str()).map(|p| Value::Str(p.to_string())).collect()),
            "replace" => {
                let from = args.get(0).map(|v| v.display()).unwrap_or_default();
                let to   = args.get(1).map(|v| v.display()).unwrap_or_default();
                Value::Str(s.replace(from.as_str(), to.as_str()))
            }
            _ => return Err(format!("unknown Str.{}", method)),
        })
    }

    fn call_list_method(&self, obj: &mut Value, method: &str, args: Vec<Value>) -> RunResult<Value> {
        let Value::List(list) = obj else { unreachable!() };
        Ok(match method {
            "len"      => Value::Int(list.len() as i64),
            "get"      => {
                let i = match args.first() { Some(Value::Int(n)) => *n as usize, _ => return Ok(Value::Null) };
                list.get(i).cloned().unwrap_or(Value::Null)
            }
            "push"     => { for a in args { list.push(a); } Value::Null }
            "pop"      => list.pop().unwrap_or(Value::Null),
            "contains" => {
                let t = args.first().unwrap_or(&Value::Null);
                Value::Bool(list.iter().any(|v| v.eq_val(t)))
            }
            "map" => {
                let name = args.first().map(|v| v.display()).unwrap_or_default();
                Value::List(list.iter().map(|v| self.call_node(&name, vec![v.clone()])).collect::<RunResult<_>>()?)
            }
            "filter" => {
                let name = args.first().map(|v| v.display()).unwrap_or_default();
                let mut out = Vec::new();
                for v in list.iter() {
                    if self.call_node(&name, vec![v.clone()])?.truthy() { out.push(v.clone()); }
                }
                Value::List(out)
            }
            _ => return Err(format!(
                "unknown List.{} — available: len, get, push, pop, contains, map, filter", method
            )),
        })
    }

    fn call_map_method(&self, obj: &mut Value, method: &str, args: Vec<Value>) -> RunResult<Value> {
        let Value::Map(map) = obj else { unreachable!() };
        let k0 = || args.first().map(|v| v.display()).unwrap_or_default();
        Ok(match method {
            "len"    => Value::Int(map.len() as i64),
            "has"    => Value::Bool(map.iter().any(|(k, _)| k == &k0())),
            "get"    => map.iter().find(|(k, _)| k == &k0()).map(|(_, v)| v.clone()).unwrap_or(Value::Null),
            "keys"   => Value::List(map.iter().map(|(k, _)| Value::Str(k.clone())).collect()),
            "values" => Value::List(map.iter().map(|(_, v)| v.clone()).collect()),
            "set"    => {
                let key = k0();
                let val = args.get(1).cloned().unwrap_or(Value::Null);
                if let Some(e) = map.iter_mut().find(|(k, _)| k == &key) { e.1 = val; }
                else { map.push((key, val)); }
                Value::Null
            }
            "delete" => {
                let key = k0();
                let before = map.len();
                map.retain(|(k, _)| k != &key);
                Value::Bool(map.len() < before)
            }
            _ => return Err(format!(
                "unknown Map.{} — available: len, has, get, keys, values, set, delete", method
            )),
        })
    }

    // ── Field access ──────────────────────────────────────────────────────────

    fn get_field(&self, obj: &Value, name: &str) -> Value {
        match obj {
            Value::Object  { fields, .. } |
            Value::Variant { fields, .. } => fields.get(name).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }

    // ── Assignment ────────────────────────────────────────────────────────────

    fn assign_target(&self, target: &Expr, op: AssignOp, rhs: Value, env: &mut Env) -> RunResult<Value> {
        match &target.kind {
            ExprKind::Ident(name) => {
                let current = env.get(name).unwrap_or(Value::Null);
                let val = self.apply_assign(op, current, rhs)?;
                if !env.assign(name, val.clone()) { env.set_local(name.clone(), val.clone()); }
                Ok(val)
            }
            ExprKind::Field { object, name } => {
                if matches!(&object.kind, ExprKind::Ident(n) if n == "shared") {
                    let mut shared = self.state.shared.lock().unwrap();
                    let current = shared.get(name).cloned().unwrap_or(Value::Null);
                    let val = self.apply_assign(op, current, rhs)?;
                    shared.insert(name.clone(), val.clone());
                    return Ok(val);
                }
                if matches!(&object.kind, ExprKind::SelfKw) {
                    if let Some(Value::Object { fields, .. }) = &mut env.self_value {
                        let current = fields.get(name).cloned().unwrap_or(Value::Null);
                        let val = self.apply_assign(op, current, rhs)?;
                        fields.insert(name.clone(), val.clone());
                        return Ok(val);
                    }
                }
                Ok(Value::Null)
            }
            ExprKind::Index { object, index } => {
                let idx = self.eval_expr(index, env)?;
                if let ExprKind::Ident(var) = &object.kind {
                    let current = env.get(var).unwrap_or(Value::Null);
                    let updated = match (current, &idx) {
                        (Value::List(mut v), Value::Int(i)) if *i >= 0 => {
                            if let Some(slot) = v.get_mut(*i as usize) { *slot = rhs.clone(); }
                            Value::List(v)
                        }
                        (Value::Map(mut m), Value::Str(k)) => {
                            if let Some(e) = m.iter_mut().find(|(key, _)| key == k) { e.1 = rhs.clone(); }
                            else { m.push((k.clone(), rhs.clone())); }
                            Value::Map(m)
                        }
                        _ => Value::Null,
                    };
                    env.assign(var, updated);
                }
                Ok(rhs)
            }
            _ => Ok(Value::Null),
        }
    }

    fn apply_assign(&self, op: AssignOp, cur: Value, rhs: Value) -> RunResult<Value> {
        match op {
            AssignOp::Assign    => Ok(rhs),
            AssignOp::AddAssign => self.eval_binary(BinOp::Add, cur, rhs),
            AssignOp::SubAssign => self.eval_binary(BinOp::Sub, cur, rhs),
            AssignOp::MulAssign => self.eval_binary(BinOp::Mul, cur, rhs),
            AssignOp::DivAssign => self.eval_binary(BinOp::Div, cur, rhs),
            AssignOp::PowAssign => self.eval_binary(BinOp::Pow, cur, rhs),
        }
    }

    // ── Binary operators (all of them) ────────────────────────────────────────

    fn eval_binary(&self, op: BinOp, l: Value, r: Value) -> RunResult<Value> {
        use Value::*;
        Ok(match op {
            BinOp::Add => match (l, r) {
                (Int(a),   Int(b))   => Int(a + b),
                (Float(a), Float(b)) => Float(a + b),
                (Int(a),   Float(b)) => Float(a as f64 + b),
                (Float(a), Int(b))   => Float(a + b as f64),
                (a, b) => Str(format!("{}{}", a.display(), b.display())),
            },
            BinOp::Sub => match (l, r) {
                (Int(a),   Int(b))   => Int(a - b),
                (Float(a), Float(b)) => Float(a - b),
                (Int(a),   Float(b)) => Float(a as f64 - b),
                (Float(a), Int(b))   => Float(a - b as f64),
                _ => Null,
            },
            BinOp::Mul => match (l, r) {
                (Int(a),   Int(b))   => Int(a * b),
                (Float(a), Float(b)) => Float(a * b),
                (Int(a),   Float(b)) => Float(a as f64 * b),
                (Float(a), Int(b))   => Float(a * b as f64),
                _ => Null,
            },
            BinOp::Div => match (l, r) {
                (Int(a),   Int(b))   if b != 0   => Int(a / b),
                (Float(a), Float(b)) if b != 0.0 => Float(a / b),
                (Int(a),   Float(b)) if b != 0.0 => Float(a as f64 / b),
                (Float(a), Int(b))   if b != 0   => Float(a / b as f64),
                _ => return Err("division by zero".into()),
            },
            BinOp::Mod => match (l, r) {
                (Int(a),   Int(b))   if b != 0   => Int(a % b),
                (Float(a), Float(b)) if b != 0.0 => Float(a % b),
                _ => Null,
            },
            BinOp::Pow => match (l, r) {
                (Int(a),   Int(b))   if b >= 0 => Int(a.pow(b as u32)),
                (Float(a), Float(b)) => Float(a.powf(b)),
                (Int(a),   Float(b)) => Float((a as f64).powf(b)),
                (Float(a), Int(b))   => Float(a.powf(b as f64)),
                _ => Null,
            },
            BinOp::Eq  => Bool(l.eq_val(&r)),
            BinOp::Ne  => Bool(!l.eq_val(&r)),
            BinOp::Lt  => Bool(self.cmp_vals(&l, &r) == std::cmp::Ordering::Less),
            BinOp::Le  => Bool(self.cmp_vals(&l, &r) != std::cmp::Ordering::Greater),
            BinOp::Gt  => Bool(self.cmp_vals(&l, &r) == std::cmp::Ordering::Greater),
            BinOp::Ge  => Bool(self.cmp_vals(&l, &r) != std::cmp::Ordering::Less),
            BinOp::And => Bool(l.truthy() && r.truthy()),
            BinOp::Or  => Bool(l.truthy() || r.truthy()),
            BinOp::BitAnd => match (l, r) { (Int(a), Int(b)) => Int(a & b), _ => Null },
            BinOp::BitOr  => match (l, r) { (Int(a), Int(b)) => Int(a | b), _ => Null },
            BinOp::BitXor => match (l, r) { (Int(a), Int(b)) => Int(a ^ b), _ => Null },
            BinOp::Shl    => match (l, r) { (Int(a), Int(b)) if b >= 0 => Int(a << b), _ => Null },
            BinOp::Shr    => match (l, r) { (Int(a), Int(b)) if b >= 0 => Int(a >> b), _ => Null },
        })
    }

    fn cmp_vals(&self, a: &Value, b: &Value) -> std::cmp::Ordering {
        match (a, b) {
            (Value::Int(a),   Value::Int(b))   => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
            (Value::Int(a),   Value::Float(b)) => (*a as f64).partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
            (Value::Float(a), Value::Int(b))   => a.partial_cmp(&(*b as f64)).unwrap_or(std::cmp::Ordering::Equal),
            (Value::Str(a),   Value::Str(b))   => a.cmp(b),
            _ => std::cmp::Ordering::Equal,
        }
    }

    // ── Pattern matching ──────────────────────────────────────────────────────

    fn pattern_matches(&self, pat: &Pattern, val: &Value, env: &mut Env) -> bool {
        match pat {
            Pattern::Wildcard => true,
            Pattern::Bind { name, ty } => {
                if let Some(TypeExpr::Named(expected)) = ty {
                    if expected != val.type_name() { return false; }
                }
                env.set_local(name.clone(), val.clone());
                true
            }
            Pattern::Literal(lit) => match (lit, val) {
                (LitPattern::Int(a),  Value::Int(b))  => a == b,
                (LitPattern::Str(a),  Value::Str(b))  => a == b,
                (LitPattern::Bool(a), Value::Bool(b)) => a == b,
                (LitPattern::Null,    Value::Null)    => true,
                _ => false,
            },
            Pattern::Range { start, end, inclusive } => match val {
                Value::Int(n) => if *inclusive { (*start..=*end).contains(n) }
                                 else          { (*start..*end).contains(n) },
                _ => false,
            },
            Pattern::Variant { variant, case, bindings } => {
                let Value::Variant { variant: gv, case: gc, fields } = val else { return false };
                if variant != gv || case != gc { return false; }
                match bindings {
                    VariantBindings::Struct(names) =>
                        for n in names { if let Some(v) = fields.get(n) { env.set_local(n.clone(), v.clone()); } },
                    VariantBindings::Tuple(names) =>
                        for (i, n) in names.iter().enumerate() {
                            if let Some(v) = fields.get(&format!("_{}", i)) { env.set_local(n.clone(), v.clone()); }
                        },
                    VariantBindings::None => {}
                }
                true
            }
        }
    }

    // ── Utilities ─────────────────────────────────────────────────────────────

    fn eval_args(&self, args: &[Expr], env: &mut Env) -> RunResult<Vec<Value>> {
        args.iter().map(|a| self.eval_expr(a, env)).collect()
    }
}

// ── Const expression evaluator (for field initializers) ───────────────────────

fn eval_const(expr: &Expr) -> RunResult<Value> {
    match &expr.kind {
        ExprKind::Int(v)    => Ok(Value::Int(*v)),
        ExprKind::Float(v)  => Ok(Value::Float(*v)),
        ExprKind::Bool(v)   => Ok(Value::Bool(*v)),
        ExprKind::Str(v)    => Ok(Value::Str(v.clone())),
        ExprKind::Null      => Ok(Value::Null),
        ExprKind::ListLit(exprs) => {
            let items = exprs.iter().map(eval_const).collect::<RunResult<_>>()?;
            Ok(Value::List(items))
        }
        ExprKind::MapLit(pairs) => {
            let mut map = Vec::new();
            for (k, v) in pairs { map.push((eval_const(k)?.display(), eval_const(v)?)); }
            Ok(Value::Map(map))
        }
        _ => Ok(Value::Null),
    }
}
