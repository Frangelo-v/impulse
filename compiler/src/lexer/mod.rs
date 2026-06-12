use logos::Logos;

#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\r\n]+")] // whitespace
#[logos(skip r"//[^\n]*")] // line comments
#[logos(skip r"/\*[^*]*\*+([^/*][^*]*\*+)*/")] // block comments
pub enum Token {
    // ── Keywords ─────────────────────────────────────────────────────────────

    // Declarations
    #[token("node")]
    KwNode,
    #[token("surge")]
    KwSurge,
    #[token("cluster")]
    KwCluster,
    #[token("enum")]
    KwEnum,
    #[token("actor")]
    KwActor,
    #[token("domain")]
    KwDomain,
    #[token("supervisor")]
    KwSupervisor,
    #[token("signal")]
    KwSignal,
    #[token("let")]
    KwLet,
    #[token("const")]
    KwConst,
    #[token("shared")]
    KwShared,
    #[token("use")]
    KwUse,

    // Surge / signal
    #[token("on")]
    KwOn,
    #[token("when")]
    KwWhen,
    #[token("start")]
    KwStart,
    #[token("stop")]
    KwStop,
    #[token("await")]
    KwAwait,
    #[token("sleep")]
    KwSleep,
    #[token("supervise")]
    KwSupervise,
    #[token("strategy")]
    KwStrategy,
    #[token("child")]
    KwChild,
    #[token("infinite")]
    KwInfinite,

    // Signal ownership
    #[token("share")]
    KwShare,
    #[token("move")]
    KwMove,

    // Signal delivery modes
    #[token("broadcast")]
    KwBroadcast,
    #[token("queue")]
    KwQueue,
    #[token("latest")]
    KwLatest,
    #[token("coalesce")]
    KwCoalesce, // alias for latest
    #[token("recursive")]
    KwRecursive,
    #[token("buffer")]
    KwBuffer,
    #[token("sample")]
    KwSample, // coalesce within N ms window

    // Signal propagation budget attributes
    #[token("budget")]
    KwBudget, // [budget: 2ms]
    #[token("max_depth")]
    KwMaxDepth, // [max_depth: 6]
    #[token("max_fanout")]
    KwMaxFanout, // [max_fanout: 128]
    #[token("scope")]
    KwScope, // [scope: local|cluster]
    #[token("local")]
    KwLocal,
    // KwCluster reuses the existing token defined at line 13 (cluster declaration keyword)

    // Surge / actor scheduling attributes
    #[token("affinity")]
    KwAffinity, // surge[affinity: "domain"]
    #[token("pin")]
    KwPin, // actor[pin: "worker_2"]
    #[token("copy")]
    KwCopy, // signal ownership: copy

    // Domain security attributes
    #[token("isolated")]
    KwIsolated, // domain[isolated]
    #[token("noncritical")]
    KwNoncritical,
    #[token("private")]
    KwPrivate,
    #[token("restricted")]
    KwRestricted,

    // Scheduler priorities
    #[token("realtime")]
    KwRealtime,
    #[token("critical")]
    KwCritical,
    #[token("idle")]
    KwIdle,
    #[token("normal")]
    KwNormal,

    // Supervision strategies
    #[token("one_for_one")]
    KwOneForOne,
    #[token("one_for_all")]
    KwOneForAll,
    #[token("rest_for_one")]
    KwRestForOne,
    #[token("escalate")]
    KwEscalate,
    #[token("max_restarts")]
    KwMaxRestarts,
    #[token("window")]
    KwWindow,

    // Control flow
    #[token("if")]
    KwIf,
    #[token("else")]
    KwElse,
    #[token("loop")]
    KwLoop,
    #[token("break")]
    KwBreak,
    #[token("match")]
    KwMatch,
    #[token("in")]
    KwIn,

    // Expressions
    #[token("pulse")]
    KwPulse,
    #[token("yield")]
    KwYield,

    // Types / values
    #[token("true")]
    KwTrue,
    #[token("false")]
    KwFalse,
    #[token("null")]
    KwNull,
    #[token("self")]
    KwSelf,
    #[token("error")]
    KwError,
    #[token("from")]
    KwFrom,

    // Logic
    #[token("and")]
    KwAnd,
    #[token("or")]
    KwOr,
    #[token("not")]
    KwNot,
    #[token("is")]
    KwIs,
    #[token("as")]
    KwAs,

    // ── Built-in type names ───────────────────────────────────────────────────
    #[token("Int")]
    TyInt,
    #[token("Int32")]
    TyInt32,
    #[token("Int8")]
    TyInt8,
    #[token("Float")]
    TyFloat,
    #[token("Float32")]
    TyFloat32,
    #[token("Bool")]
    TyBool,
    #[token("Str")]
    TyStr,
    #[token("Byte")]
    TyByte,
    #[token("Null")]
    TyNull,
    #[token("Trace")]
    TyTrace,

    // ── Literals ─────────────────────────────────────────────────────────────

    // Hex: 0xFF
    #[regex(r"0x[0-9a-fA-F][0-9a-fA-F_]*",
        |lex| i64::from_str_radix(&lex.slice()[2..].replace('_', ""), 16).ok())]
    LitHexInt(i64),

    // Binary: 0b1010
    #[regex(r"0b[01][01_]*",
        |lex| i64::from_str_radix(&lex.slice()[2..].replace('_', ""), 2).ok())]
    LitBinInt(i64),

    // Octal: 0o777
    #[regex(r"0o[0-7][0-7_]*",
        |lex| i64::from_str_radix(&lex.slice()[2..].replace('_', ""), 8).ok())]
    LitOctInt(i64),

    // Decimal integer
    #[regex(r"[0-9][0-9_]*",
        |lex| lex.slice().replace('_', "").parse::<i64>().ok())]
    LitInt(i64),

    // Float
    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9]+)?",
        |lex| lex.slice().replace('_', "").parse::<f64>().ok())]
    LitFloat(f64),

    // String — escapes resolved here so every consumer sees the real text
    #[regex(r#""([^"\\]|\\.)*""#, |lex| {
        let s = lex.slice();
        Some(unescape(&s[1..s.len()-1]))
    })]
    LitString(String),

    // ── Identifier ───────────────────────────────────────────────────────────
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Ident(String),

    // ── Arithmetic operators ──────────────────────────────────────────────────
    #[token("**")]
    StarStar, // exponentiation (must come before *)
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,

    // ── Bitwise operators ─────────────────────────────────────────────────────
    #[token("<<")]
    LtLt,
    #[token(">>")]
    GtGt,
    #[token("&")]
    Amp,
    #[token("~")]
    Tilde, // bitwise XOR

    // ── Comparison operators ──────────────────────────────────────────────────
    #[token("==")]
    EqEq,
    #[token("!=")]
    BangEq,
    #[token("<=")]
    LtEq,
    #[token(">=")]
    GtEq,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,

    // ── Assignment operators ──────────────────────────────────────────────────
    #[token("**=")]
    StarStarEq,
    #[token("+=")]
    PlusEq,
    #[token("-=")]
    MinusEq,
    #[token("*=")]
    StarEq,
    #[token("/=")]
    SlashEq,
    #[token("=")]
    Eq,

    // ── Other operators ───────────────────────────────────────────────────────
    #[token("|>")]
    PipeArrow, // pipe operator
    #[token("->")]
    Arrow, // return type / match arm
    #[token("<-")]
    TraceRecv, // trace receive / send
    #[token("..=")]
    RangeInc,
    #[token("..")]
    Range,
    #[token("??")]
    DoubleQuestion,
    #[token("?")]
    Question,
    #[token("|")]
    Pipe,
    #[token("::")]
    ColonColon, // domain::signal separator

    // ── Delimiters ────────────────────────────────────────────────────────────
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token(";")]
    Semi,
    #[token(".")]
    Dot,
    #[token("@")]
    At,
}

// ── String escapes ────────────────────────────────────────────────────────────

fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n')  => out.push('\n'),
            Some('t')  => out.push('\t'),
            Some('r')  => out.push('\r'),
            Some('"')  => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('0')  => out.push('\0'),
            // Escape desconocido: se conserva tal cual (indulgente)
            Some(other) => { out.push('\\'); out.push(other); }
            None        => out.push('\\'),
        }
    }
    out
}

// ── Human-readable token names for error messages ─────────────────────────────

/// Describe a token the way a user wrote it: `'{'`, `keyword 'signal'`,
/// `identifier 'foo'` — never the internal Debug name.
pub fn describe(tok: &Token) -> String {
    use Token::*;
    let sym = match tok {
        LitInt(_) | LitHexInt(_) | LitBinInt(_) | LitOctInt(_) | LitFloat(_) =>
            return "a number".into(),
        LitString(_) => return "a string".into(),
        Ident(n)     => return format!("identifier '{}'", n),
        StarStar => "**", Plus => "+", Minus => "-", Star => "*", Slash => "/",
        Percent => "%", LtLt => "<<", GtGt => ">>", Amp => "&", Tilde => "~",
        EqEq => "==", BangEq => "!=", LtEq => "<=", GtEq => ">=", Lt => "<", Gt => ">",
        StarStarEq => "**=", PlusEq => "+=", MinusEq => "-=", StarEq => "*=",
        SlashEq => "/=", Eq => "=", PipeArrow => "|>", Arrow => "->", TraceRecv => "<-",
        RangeInc => "..=", Range => "..", DoubleQuestion => "??", Question => "?",
        Pipe => "|", ColonColon => "::", LBrace => "{", RBrace => "}", LParen => "(",
        RParen => ")", LBracket => "[", RBracket => "]", Comma => ",", Colon => ":",
        Semi => ";", Dot => ".", At => "@",
        other => {
            // Keywords: KwMaxFanout -> keyword 'max_fanout'
            let dbg = format!("{:?}", other);
            if let Some(rest) = dbg.strip_prefix("Kw") {
                return format!("keyword '{}'", camel_to_snake(rest));
            }
            return format!("'{}'", dbg);
        }
    };
    format!("'{}'", sym)
}

/// Describe an optional token; `None` means the source ended.
pub fn describe_opt(tok: Option<&Token>) -> String {
    match tok {
        Some(t) => describe(t),
        None    => "end of file".into(),
    }
}

fn camel_to_snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

// ── Spanned token ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Spanned<T> {
    pub node: T,
    pub span: std::ops::Range<usize>,
}

pub type SpannedToken = Spanned<Token>;

pub fn tokenize(source: &str) -> Vec<SpannedToken> {
    Token::lexer(source)
        .spanned()
        .filter_map(|(result, span)| result.ok().map(|tok| Spanned { node: tok, span }))
        .collect()
}
