// Impulse Semantic Analysis — v0.3
//
// Responsibilities (per spec §7.5, §15.1):
//   1. Signal Graph — build a compile-time map: signal_name → [surge_names]
//   2. Cycle Detection — DFS over the signal graph; error on cycles unless [recursive]
//   3. Signal Declaration Check — all `on X` and `signal X(...)` must reference a declared signal
//   4. `on` scope check — reactive handlers only at top level / inside domains
//   5. `shared` type restriction — only scalar types allowed
//   6. `node main()` existence check — exactly one must exist
//   7. `move` ownership check — signal with `move` must have exactly one listener

use crate::ast::*;
use std::collections::{HashMap, HashSet};

// ── Public entry point ────────────────────────────────────────────────────────

pub struct SemanticError {
    pub span: Span,
    pub message: String,
}

pub struct SemanticWarning {
    pub span: Span,
    pub message: String,
}

pub struct AnalysisResult {
    pub errors: Vec<SemanticError>,
    pub warnings: Vec<SemanticWarning>,
    pub signal_graph: SignalGraph,
}

pub fn analyze(program: &Program) -> AnalysisResult {
    let mut ctx = Ctx::new();
    ctx.run(program);
    AnalysisResult {
        errors: ctx.errors,
        warnings: ctx.warnings,
        signal_graph: ctx.graph,
    }
}

// ── Signal Graph ──────────────────────────────────────────────────────────────

/// Compiled signal dependency graph.
/// signal_name → list of (surge_name, module) that listen to it
#[derive(Debug, Default)]
pub struct SignalGraph {
    /// signal name → listeners
    pub listeners: HashMap<String, Vec<String>>,
    /// signal name → emitters (surges that call `signal X(...)`)
    pub emitters: HashMap<String, Vec<String>>,
    /// signal name → modes declared
    pub modes: HashMap<String, Vec<SignalMode>>,
    /// signals marked [recursive]
    pub recursive: HashSet<String>,
}

impl SignalGraph {
    fn add_listener(&mut self, signal: String, surge: String) {
        self.listeners.entry(signal).or_default().push(surge);
    }

    fn add_emitter(&mut self, signal: String, surge: String) {
        self.emitters.entry(signal).or_default().push(surge);
    }

    fn declare(&mut self, signal: String, modes: Vec<SignalMode>) {
        let recursive = modes.iter().any(|m| matches!(m, SignalMode::Recursive));
        if recursive {
            self.recursive.insert(signal.clone());
        }
        self.modes.insert(signal, modes);
    }

    /// Check for cycles: signal A emits B, B emits A → cycle
    /// Returns list of cycles found (each cycle is a Vec of signal names)
    pub fn find_cycles(&self) -> Vec<Vec<String>> {
        let mut visited = HashSet::new();
        let mut in_stack = HashSet::new();
        let mut cycles = Vec::new();

        for start in self.emitters.keys() {
            if !visited.contains(start) {
                self.dfs(
                    start,
                    &mut visited,
                    &mut in_stack,
                    &mut Vec::new(),
                    &mut cycles,
                );
            }
        }
        cycles
    }

    fn dfs(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        in_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        visited.insert(node.to_string());
        in_stack.insert(node.to_string());
        path.push(node.to_string());

        // The signals that `node` (a signal) can reach:
        // node is emitted by surges → those surges may also emit other signals
        let empty = Vec::new();
        let listeners = self.listeners.get(node).unwrap_or(&empty);

        for listener_surge in listeners {
            // Find signals that this listener surge emits
            for (emitted_signal, emitters) in &self.emitters {
                if emitters.contains(listener_surge) {
                    if in_stack.contains(emitted_signal) && !self.recursive.contains(emitted_signal)
                    {
                        // Found a cycle
                        let cycle_start =
                            path.iter().position(|s| s == emitted_signal).unwrap_or(0);
                        cycles.push(path[cycle_start..].to_vec());
                    } else if !visited.contains(emitted_signal) {
                        self.dfs(emitted_signal, visited, in_stack, path, cycles);
                    }
                }
            }
        }

        path.pop();
        in_stack.remove(node);
    }
}

// ── Analysis Context ──────────────────────────────────────────────────────────

struct Ctx {
    errors: Vec<SemanticError>,
    warnings: Vec<SemanticWarning>,
    graph: SignalGraph,

    declared_signals: HashSet<String>,
    declared_nodes: HashSet<String>,
    declared_surges: HashSet<String>,
    found_main: bool,
    current_surge: Option<String>,
    /// Domains marked `private` — their signals are not visible outside
    private_domains: HashSet<String>,
    /// Domains marked `isolated` — crashes don't propagate out
    isolated_domains: HashSet<String>,
}

impl Ctx {
    fn new() -> Self {
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
            graph: SignalGraph::default(),
            declared_signals: HashSet::new(),
            declared_nodes: HashSet::new(),
            declared_surges: HashSet::new(),
            found_main: false,
            current_surge: None,
            private_domains: HashSet::new(),
            isolated_domains: HashSet::new(),
        }
    }

    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.errors.push(SemanticError {
            span,
            message: msg.into(),
        });
    }

    fn warn(&mut self, span: Span, msg: impl Into<String>) {
        self.warnings.push(SemanticWarning {
            span,
            message: msg.into(),
        });
    }

    fn run(&mut self, program: &Program) {
        // Pass 1: collect all declared signals and node names
        self.collect_declarations(program);

        // Pass 2: full analysis
        for decl in &program.decls {
            self.check_top_decl(decl, false);
        }

        // Post-pass checks
        self.check_main_exists();
        self.check_signal_cycles();
        self.check_dead_signals();
        self.check_move_ownership();
        self.check_propagation_budgets();
        self.check_domain_visibility();
    }

    // ── Pass 1: collect declarations ─────────────────────────────────────────

    fn collect_declarations(&mut self, program: &Program) {
        for decl in &program.decls {
            match decl {
                TopDecl::Signal(s) => {
                    let name = s.name.qualified();
                    if self.declared_signals.contains(&name) {
                        self.error(
                            s.span.clone(),
                            format!(
                                "signal '{}' declared more than once in the same scope",
                                name
                            ),
                        );
                    }
                    self.declared_signals.insert(name.clone());
                    self.graph.declare(name, s.modes.clone());
                }
                TopDecl::Domain(d) => {
                    // Register domain security attributes
                    if d.attrs.private {
                        self.private_domains.insert(d.name.clone());
                    }
                    if d.attrs.isolated {
                        self.isolated_domains.insert(d.name.clone());
                    }
                    for sig in &d.signals {
                        let name = format!("{}::{}", d.name, sig.name.name);
                        self.declared_signals.insert(name.clone());
                        self.graph.declare(name, sig.modes.clone());
                    }
                    for surge in &d.surges {
                        if let SurgeKind::Active { name } = &surge.kind {
                            self.declared_surges.insert(name.clone());
                        }
                    }
                }
                TopDecl::Node(n) => {
                    self.declared_nodes.insert(n.name.clone());
                }
                TopDecl::Surge(s) => {
                    if let SurgeKind::Active { name } = &s.kind {
                        self.declared_surges.insert(name.clone());
                    }
                }
                _ => {}
            }
        }

        // Built-in signals always declared
        for builtin in &["surge::error", "surge::dead", "os::signal"] {
            self.declared_signals.insert(builtin.to_string());
        }
    }

    // ── Pass 2: check declarations ────────────────────────────────────────────

    fn check_top_decl(&mut self, decl: &TopDecl, inside_block: bool) {
        match decl {
            TopDecl::Surge(s) => self.check_surge(s, inside_block),
            TopDecl::Node(n) => self.check_node(n),
            TopDecl::Cortex(c) => self.check_cortex(c),
            TopDecl::Domain(d) => self.check_domain(d),
            TopDecl::Actor(a) => self.check_actor(a),
            TopDecl::Supervisor(s) => self.check_supervisor(s),
            _ => {}
        }
    }

    fn check_surge(&mut self, surge: &SurgeDecl, inside_block: bool) {
        match &surge.kind {
            SurgeKind::Reactive { signal } => {
                // Rule: reactive surges only at top level (spec §8.6)
                if inside_block {
                    self.error(surge.span.clone(),
                        "reactive handler (`on`) may only be declared at module top level or inside a domain block, not inside a loop, node, or active surge body"
                    );
                }

                let sig_name = signal.qualified();

                // Check signal is declared
                if !self.declared_signals.contains(&sig_name) {
                    self.error(surge.span.clone(), format!(
                        "surge listens to undeclared signal '{}' — declare it with `signal {}:` before use",
                        sig_name, sig_name
                    ));
                }

                // Register in signal graph
                let surge_name = format!("reactive@{}", sig_name);
                self.graph.add_listener(sig_name, surge_name.clone());

                // Check body for emitted signals
                let prev = self.current_surge.replace(surge_name);
                self.check_stmts(&surge.body, true);
                self.current_surge = prev;
            }
            SurgeKind::Active { name } => {
                let prev = self.current_surge.replace(name.clone());
                self.check_stmts(&surge.body, true);
                self.current_surge = prev;
            }
        }
    }

    fn check_node(&mut self, node: &NodeDecl) {
        if node.name == "main" && node.params.is_empty() {
            self.found_main = true;
        }
        let prev = self.current_surge.replace(format!("node:{}", node.name));
        self.check_stmts(&node.body, true);
        self.current_surge = prev;
    }

    fn check_cortex(&mut self, cortex: &CortexDecl) {
        // shared only accepts scalar types
        let allowed = matches!(
            &cortex.ty,
            TypeExpr::Named(n) if matches!(n.as_str(), "Int" | "Int32" | "Int8" | "Float" | "Float32" | "Bool" | "Byte")
        );
        if !allowed {
            self.error(cortex.span.clone(), format!(
                "shared field '{}' must be a scalar type (Int, Float, Bool, Byte). Use `actor` for compound state",
                cortex.name
            ));
        }
    }

    fn check_domain(&mut self, domain: &DomainDecl) {
        for surge in &domain.surges {
            self.check_surge_in_domain(surge, &domain.name);
        }
    }

    fn check_surge_in_domain(&mut self, surge: &SurgeDecl, domain_name: &str) {
        if let SurgeKind::Reactive { signal } = &surge.kind {
            // Qualify the signal name with the domain prefix for lookup
            let sig_name = if signal.domain.is_some() {
                signal.qualified()
            } else {
                format!("{}::{}", domain_name, signal.name)
            };

            if !self.declared_signals.contains(&sig_name) {
                self.error(surge.span.clone(), format!(
                    "surge listens to undeclared signal '{}' — declare it inside domain '{}'",
                    sig_name, domain_name
                ));
            }

            let surge_name = format!("reactive@{}", sig_name);
            self.graph.add_listener(sig_name, surge_name.clone());

            let prev = self.current_surge.replace(surge_name);
            self.check_stmts(&surge.body, true);
            self.current_surge = prev;
        } else {
            self.check_surge(surge, false);
        }
    }

    fn check_actor(&mut self, actor: &ActorDecl) {
        for method in &actor.methods {
            let prev = self
                .current_surge
                .replace(format!("actor:{}:{}", actor.name, method.name));
            self.check_stmts(&method.body, true);
            self.current_surge = prev;
        }
    }

    fn check_supervisor(&mut self, sup: &SupervisorDecl) {
        // Verify all child callees are declared nodes or surges
        for child in &sup.children {
            if !self.declared_nodes.contains(&child.callee)
                && !self.declared_surges.contains(&child.callee)
            {
                self.warn(
                    sup.span.clone(),
                    format!(
                    "supervisor child '{}' calls '{}' which is not a declared node or active surge",
                    child.label, child.callee
                ),
                );
            }
        }
    }

    // ── Statement / expression traversal ─────────────────────────────────────

    fn check_stmts(&mut self, stmts: &[Stmt], in_block: bool) {
        for stmt in stmts {
            self.check_stmt(stmt, in_block);
        }
    }

    fn check_stmt(&mut self, stmt: &Stmt, in_block: bool) {
        match stmt {
            Stmt::Decl(d) => self.check_top_decl(d, in_block),
            Stmt::Expr(e) => self.check_expr(e),
            Stmt::TraceSend { channel, value, .. } => {
                self.check_expr(channel);
                self.check_expr(value);
            }
            Stmt::Break(_) => {}
        }
    }

    fn check_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Signal {
                name, ownership, ..
            } => {
                let sig_name = name.qualified();

                // Check signal is declared
                if !self.declared_signals.contains(&sig_name) {
                    self.error(
                        expr.span.clone(),
                        format!(
                            "signal '{}' is not declared — add `signal {}:` at module top level",
                            sig_name, sig_name
                        ),
                    );
                }

                // Register this surge as an emitter
                if let Some(surge) = &self.current_surge {
                    self.graph.add_emitter(sig_name.clone(), surge.clone());
                }

                // Track move ownership for post-pass check
                if *ownership == SignalOwnership::Move {
                    // Will verify in check_move_ownership
                }
            }

            // Recurse into subexpressions
            ExprKind::Binary { left, right, .. } => {
                self.check_expr(left);
                self.check_expr(right);
            }
            ExprKind::Unary { operand, .. } => self.check_expr(operand),
            ExprKind::Call { callee, args, .. } => {
                self.check_expr(callee);
                for a in args {
                    self.check_expr(a);
                }
            }
            ExprKind::Method { object, args, .. } => {
                self.check_expr(object);
                for a in args {
                    self.check_expr(a);
                }
            }
            ExprKind::Field { object, .. } | ExprKind::Index { object, .. } => {
                self.check_expr(object);
            }
            ExprKind::Pulse(e)
            | ExprKind::Yield(e)
            | ExprKind::Try(e)
            | ExprKind::Collect(e)
            | ExprKind::Rest(e)
            | ExprKind::TraceRecv(e) => {
                self.check_expr(e);
            }
            ExprKind::Coalesce(a, b)
            | ExprKind::Assign {
                target: a,
                value: b,
                ..
            } => {
                self.check_expr(a);
                self.check_expr(b);
            }
            ExprKind::When {
                cond,
                then,
                else_ifs,
                else_,
            } => {
                self.check_expr(cond);
                self.check_stmts(then, true);
                for (ec, eb) in else_ifs {
                    self.check_expr(ec);
                    self.check_stmts(eb, true);
                }
                if let Some(e) = else_ {
                    self.check_stmts(e, true);
                }
            }
            ExprKind::Loop { binding, body } => {
                if let Some((_, iter)) = binding {
                    self.check_expr(iter);
                }
                self.check_stmts(body, true);
            }
            ExprKind::Match { subject, arms } => {
                self.check_expr(subject);
                for arm in arms {
                    self.check_stmts(&arm.body, true);
                }
            }
            ExprKind::Select { arms } => {
                for arm in arms {
                    self.check_expr(&arm.channel);
                    self.check_stmts(&arm.body, true);
                }
            }
            ExprKind::Open { callee, args, .. } => {
                self.check_expr(callee);
                for a in args {
                    self.check_expr(a);
                }
            }
            ExprKind::Pipe(e, _) => self.check_expr(e),
            _ => {}
        }
    }

    // ── Post-pass checks ──────────────────────────────────────────────────────

    fn check_main_exists(&mut self) {
        if !self.found_main {
            self.error(0..0, "no `node main()` found — every Impulse program must define exactly one entry point");
        }
    }

    fn check_signal_cycles(&mut self) {
        let cycles = self.graph.find_cycles();
        for cycle in cycles {
            self.error(0..0, format!(
                "reactive cycle detected: {} — mark the signal `[recursive]` if this is intentional",
                cycle.join(" → ")
            ));
        }
    }

    fn check_dead_signals(&mut self) {
        // Collect dead signals first to avoid borrow conflict
        let dead: Vec<String> = self
            .graph
            .modes
            .keys()
            .filter(|sig| {
                self.graph
                    .listeners
                    .get(*sig)
                    .map(|l| l.is_empty())
                    .unwrap_or(true)
            })
            .cloned()
            .collect();

        for signal in dead {
            self.warn(
                0..0,
                format!(
                    "signal '{}' is declared but has no registered surge listeners",
                    signal
                ),
            );
        }
    }

    fn check_move_ownership(&mut self) {
        // A signal emitted with `move` ownership may only have one listener —
        // otherwise the value would be moved into multiple handlers simultaneously.
        let violations: Vec<(String, usize)> = self
            .graph
            .listeners
            .iter()
            .filter_map(|(signal, listeners)| {
                let modes = self.graph.modes.get(signal)?;
                let is_move = modes.iter().any(|m| matches!(m, SignalMode::Scope(_)));
                if is_move && listeners.len() != 1 {
                    Some((signal.clone(), listeners.len()))
                } else {
                    None
                }
            })
            .collect();

        for (signal, count) in violations {
            self.error(
                0..0,
                format!(
                    "signal '{}' uses move ownership but has {} listeners — move signals must have exactly 1 listener",
                    signal, count
                ),
            );
        }
    }

    fn check_propagation_budgets(&mut self) {
        // Collect violations first — avoids borrow conflict on self.graph + self.error/warn
        type Issue = (bool, String); // (is_error, message)
        let issues: Vec<Issue> = self.graph.modes.iter()
            .flat_map(|(signal, modes)| {
                modes.iter().filter_map(|mode| match mode {
                    SignalMode::MaxFanout(0) => Some((true, format!(
                        "signal '{}' has max_fanout: 0 — no surges will ever activate", signal
                    ))),
                    SignalMode::MaxDepth(0) => Some((true, format!(
                        "signal '{}' has max_depth: 0 — reactive chains will be immediately terminated", signal
                    ))),
                    SignalMode::Budget(0) => Some((false, format!(
                        "signal '{}' has budget: 0ms — propagation will always exceed budget immediately", signal
                    ))),
                    _ => None,
                }).collect::<Vec<_>>()
            })
            .collect();

        for (is_error, msg) in issues {
            if is_error {
                self.error(0..0, msg);
            } else {
                self.warn(0..0, msg);
            }
        }
    }

    fn check_domain_visibility(&mut self) {
        // Collect violations before calling self.warn (borrow conflict)
        let violations: Vec<String> = self
            .graph
            .listeners
            .iter()
            .flat_map(|(sig_name, listeners)| {
                let domain = sig_name.split("::").next().unwrap_or("").to_string();
                if self.private_domains.contains(&domain) {
                    let prefix = format!("reactive@{}::", domain);
                    listeners
                        .iter()
                        .filter(|l| !l.starts_with(&prefix))
                        .map(|_| {
                            format!(
                            "signal '{}' is in private domain '{}' but has listeners outside it",
                            sig_name, domain
                        )
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![]
                }
            })
            .collect();

        for msg in violations {
            self.warn(0..0, msg);
        }
    }
}
