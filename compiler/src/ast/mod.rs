#![allow(dead_code)]

pub type Span = std::ops::Range<usize>;

// ── Program ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Program {
    pub decls: Vec<TopDecl>,
}

// ── Top-level declarations ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TopDecl {
    Node(NodeDecl),
    Surge(SurgeDecl),
    Cluster(ClusterDecl),
    Variant(VariantDecl),
    Actor(ActorDecl),
    Domain(DomainDecl),
    Supervisor(SupervisorDecl),
    Signal(SignalDecl),
    Charge(ChargeDecl),
    Cortex(CortexDecl),
    Link(LinkDecl),
}

// ── Node ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct NodeDecl {
    pub span: Span,
    pub name: String,
    pub generics: Vec<String>,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub body: Vec<Stmt>,
}

// ── Surge ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SurgeDecl {
    pub span: Span,
    pub kind: SurgeKind,
    pub attrs: SurgeAttrs, // supervise + affinity
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum SurgeKind {
    /// `on [domain::]signal_name(...)`
    Reactive { signal: SignalRef },
    /// `surge name(...)`
    Active { name: String },
}

#[derive(Debug, Clone)]
pub enum SuperviseCount {
    N(u32),
    Infinite,
}

// ── Signal declaration ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SignalDecl {
    pub span: Span,
    pub name: SignalRef,
    pub ty: TypeExpr,
    pub modes: Vec<SignalMode>,
}

/// A signal reference: either `name` or `domain::name`
#[derive(Debug, Clone)]
pub struct SignalRef {
    pub domain: Option<String>,
    pub name: String,
}

impl SignalRef {
    pub fn qualified(&self) -> String {
        match &self.domain {
            Some(d) => format!("{}::{}", d, self.name),
            None => self.name.clone(),
        }
    }
}

/// Delivery + propagation attributes on a signal declaration
#[derive(Debug, Clone)]
pub enum SignalMode {
    // Delivery modes
    Broadcast,
    Queue,
    Latest,   // coalesce — keep only newest
    Coalesce, // alias for Latest
    Recursive,
    Buffer(u64), // ring buffer, cap N
    Sample(u64), // coalesce within N-ms window, then dispatch once

    // Propagation budget attributes
    Budget(u64),    // max CPU time in ms for the whole propagation chain
    MaxDepth(u32),  // max recursive signal depth
    MaxFanout(u32), // max concurrent surge activations per emission

    // Scope (regional propagation)
    Scope(PropScope), // local = stays in domain, cluster = can cross domains
}

/// Where a signal propagates
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropScope {
    Local,   // stays within its domain scheduler
    Cluster, // can cross domain boundaries (default)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SignalPriority {
    Idle,
    Normal,
    Critical,
    Realtime,
}

/// Security and isolation attributes on a domain
#[derive(Debug, Clone, Default)]
pub struct DomainAttrs {
    /// Crash inside this domain cannot propagate out to other domains
    pub isolated: bool,
    /// If this domain crashes entirely, the runtime continues without it
    pub noncritical: bool,
    /// Signals in this domain are not visible outside it
    pub private: bool,
    /// External domains need explicit `use` permission to emit into this domain
    pub restricted: bool,
}

/// Scheduling attributes on a surge declaration
#[derive(Debug, Clone, Default)]
pub struct SurgeAttrs {
    pub supervise: Option<SuperviseCount>,
    /// Prefer running on a specific domain's worker thread pool
    pub affinity: Option<String>,
}

/// Scheduling attributes on an actor declaration
#[derive(Debug, Clone, Default)]
pub struct ActorAttrs {
    /// Stay on a specific named worker thread (prevents migration)
    pub pin: Option<String>,
}

// ── Cluster ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ClusterDecl {
    pub span: Span,
    pub name: String,
    pub generics: Vec<String>,
    pub fields: Vec<Field>,
    pub methods: Vec<NodeDecl>,
}

// ── Variant (sum type) ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct VariantDecl {
    pub span: Span,
    pub name: String,
    pub generics: Vec<String>,
    pub cases: Vec<VariantCase>,
}

#[derive(Debug, Clone)]
pub struct VariantCase {
    pub name: String,
    pub kind: VariantCaseKind,
}

#[derive(Debug, Clone)]
pub enum VariantCaseKind {
    Unit,
    Tuple(Vec<TypeExpr>),
    Struct(Vec<Field>),
}

// ── Actor ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ActorDecl {
    pub span: Span,
    pub name: String,
    pub attrs: ActorAttrs,
    pub fields: Vec<ChargeDecl>, // actor fields have initializers like let x: Int = 0
    pub methods: Vec<NodeDecl>,
}

// ── Domain ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DomainDecl {
    pub span: Span,
    pub name: String,
    pub attrs: DomainAttrs, // isolated, noncritical, private, restricted
    pub signals: Vec<SignalDecl>,
    pub surges: Vec<SurgeDecl>,
}

// ── Supervisor ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SupervisorDecl {
    pub span: Span,
    pub name: String,
    pub strategy: SupervisionStrategy,
    pub children: Vec<SupervisorChild>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SupervisionStrategy {
    OneForOne,
    OneForAll,
    RestForOne,
    Escalate,
}

#[derive(Debug, Clone)]
pub struct SupervisorChild {
    pub label: String,
    pub callee: String,
    pub args: Vec<Expr>,
    pub max_restarts: Option<SuperviseCount>,
    pub window_ms: Option<u64>,
}

// ── Charge / Cortex / Link ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ChargeDecl {
    pub span: Span,
    pub name: String,
    pub immutable: bool,
    pub ty: Option<TypeExpr>,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub struct CortexDecl {
    pub span: Span,
    pub name: String,
    pub ty: TypeExpr,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub struct LinkDecl {
    pub span: Span,
    pub names: Vec<String>,
    pub source: String,
}

// ── Shared building blocks ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: TypeExpr,
}

// ── Type expressions ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TypeExpr {
    Named(String),
    Generic(String, Vec<TypeExpr>),
    List(Box<TypeExpr>),
    Map(Box<TypeExpr>, Box<TypeExpr>),
    Tuple(Vec<TypeExpr>),
    Optional(Box<TypeExpr>),
    Result(Box<TypeExpr>),
    Trace {
        inner: Box<TypeExpr>,
        cap: Option<u64>,
    },
    NodeType {
        params: Vec<TypeExpr>,
        ret: Box<TypeExpr>,
    },
}

// ── Statements ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Stmt {
    Decl(TopDecl),
    Expr(Expr),
    TraceSend {
        channel: Box<Expr>,
        value: Box<Expr>,
        span: Span,
    },
    Break(Span),
}

// ── Expressions ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Expr {
    pub span: Span,
    pub kind: ExprKind,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    // Literals
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Null,

    // References
    Ident(String),
    SelfKw,

    // Binary / unary
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Unary {
        op: UnOp,
        operand: Box<Expr>,
    },

    // Assignment
    Assign {
        target: Box<Expr>,
        op: AssignOp,
        value: Box<Expr>,
    },

    // Access
    Field {
        object: Box<Expr>,
        name: String,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Method {
        object: Box<Expr>,
        name: String,
        args: Vec<Expr>,
    },

    // Optionals
    Try(Box<Expr>),
    Coalesce(Box<Expr>, Box<Expr>),

    // Pipe
    Pipe(Box<Expr>, String),

    // Domain-qualified name: domain::name
    DomainRef {
        domain: String,
        name: String,
    },

    // Constructors
    ClusterLit {
        name: String,
        fields: Vec<(String, Expr)>,
    },
    VariantLit {
        variant: String,
        case: String,
        payload: VariantPayload,
    },
    ListLit(Vec<Expr>),
    MapLit(Vec<(Expr, Expr)>),
    TupleLit(Vec<Expr>),

    // Node return
    Pulse(Box<Expr>),
    Yield(Box<Expr>),

    // Concurrency
    Signal {
        name: SignalRef,
        args: Vec<Expr>,
        ownership: SignalOwnership,
        priority: SignalPriority,
    },
    Open {
        label: Option<String>,
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    OpenSupervisor(String),
    Close(String),
    Collect(Box<Expr>),
    Rest(Box<Expr>),

    // Trace
    TraceRecv(Box<Expr>),

    // Control flow
    When {
        cond: Box<Expr>,
        then: Vec<Stmt>,
        else_ifs: Vec<(Expr, Vec<Stmt>)>,
        else_: Option<Vec<Stmt>>,
    },
    Loop {
        binding: Option<(String, Box<Expr>)>,
        body: Vec<Stmt>,
    },
    Match {
        subject: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    Select {
        arms: Vec<SelectArm>,
    },
    Break,
}

#[derive(Debug, Clone)]
pub enum VariantPayload {
    None,
    Tuple(Vec<Expr>),
    Struct(Vec<(String, Expr)>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SignalOwnership {
    Clone, // default
    Share, // Arc immutable ref
    Move,  // ownership transfer
}

// ── Match / Select arms ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct SelectArm {
    pub binding: String,
    pub channel: Expr,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard,
    Literal(LitPattern),
    Range {
        start: i64,
        end: i64,
        inclusive: bool,
    },
    Bind {
        name: String,
        ty: Option<TypeExpr>,
    },
    Variant {
        variant: String,
        case: String,
        bindings: VariantBindings,
    },
}

#[derive(Debug, Clone)]
pub enum VariantBindings {
    None,
    Tuple(Vec<String>),
    Struct(Vec<String>),
}

#[derive(Debug, Clone)]
pub enum LitPattern {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Null,
}

// ── Operators ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AssignOp {
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    PowAssign,
}
