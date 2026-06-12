use crate::ast::*;
use crate::lexer::{SpannedToken, Token};

pub type ParseError = String;
pub type ParseResult<T> = Result<T, ParseError>;

pub fn parse(tokens: Vec<SpannedToken>, _source: &str) -> ParseResult<Program> {
    let mut parser = Parser::new(tokens);
    match parser.parse_program() {
        Ok(program) => Ok(program),
        Err(err) => {
            let span = parser.span();
            Err(format!("{} @{}..{}", err, span.start, span.end))
        }
    }
}

struct Parser {
    tokens: Vec<SpannedToken>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<SpannedToken>) -> Self {
        Self { tokens, pos: 0 }
    }

    // ── Cursor helpers ────────────────────────────────────────────────────────

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|t| &t.node)
    }

    fn peek2(&self) -> Option<&Token> {
        self.tokens.get(self.pos + 1).map(|t| &t.node)
    }

    fn peek3(&self) -> Option<&Token> {
        self.tokens.get(self.pos + 2).map(|t| &t.node)
    }

    fn span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|t| t.span.clone())
            .or_else(|| {
                self.tokens
                    .last()
                    .map(|t| t.span.end..t.span.end.saturating_add(1))
            })
            .unwrap_or(0..0)
    }

    fn advance(&mut self) -> Option<Token> {
        let tok = self.tokens.get(self.pos).map(|t| t.node.clone());
        self.pos += 1;
        tok
    }

    fn expect(&mut self, expected: &Token) -> ParseResult<()> {
        match self.peek() {
            Some(t) if t == expected => {
                self.advance();
                Ok(())
            }
            other => Err(format!(
                "expected {}, found {}",
                crate::lexer::describe(expected),
                crate::lexer::describe_opt(other)
            )),
        }
    }

    fn expect_ident(&mut self) -> ParseResult<String> {
        match self.peek().cloned() {
            Some(Token::Ident(n)) => {
                self.advance();
                Ok(n)
            }
            other => Err(format!(
                "expected a name, found {}",
                crate::lexer::describe_opt(other.as_ref())
            )),
        }
    }

    fn expect_string(&mut self) -> ParseResult<String> {
        match self.peek().cloned() {
            Some(Token::LitString(s)) => {
                self.advance();
                Ok(s)
            }
            other => Err(format!(
                "expected a string, found {}",
                crate::lexer::describe_opt(other.as_ref())
            )),
        }
    }

    fn eat(&mut self, tok: &Token) -> bool {
        if self.peek() == Some(tok) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    // ── Program ───────────────────────────────────────────────────────────────

    fn parse_program(&mut self) -> ParseResult<Program> {
        let mut decls = Vec::new();
        while !self.at_end() {
            self.eat(&Token::Semi);
            if self.at_end() {
                break;
            }
            decls.push(self.parse_top_decl()?);
        }
        Ok(Program { decls })
    }

    // ── Top-level declarations ────────────────────────────────────────────────

    fn parse_top_decl(&mut self) -> ParseResult<TopDecl> {
        match self.peek() {
            Some(Token::KwNode) => Ok(TopDecl::Node(self.parse_node_decl()?)),
            Some(Token::KwSurge) => Ok(TopDecl::Surge(self.parse_surge_decl()?)),
            Some(Token::KwWhen) | Some(Token::KwOn) => Ok(TopDecl::Surge(self.parse_on_decl()?)),
            Some(Token::KwCluster) => Ok(TopDecl::Cluster(self.parse_cluster_decl()?)),
            Some(Token::KwEnum) => Ok(TopDecl::Variant(self.parse_variant_decl()?)),
            Some(Token::KwActor) => Ok(TopDecl::Actor(self.parse_actor_decl()?)),
            Some(Token::KwDomain) => Ok(TopDecl::Domain(self.parse_domain_decl()?)),
            Some(Token::KwSupervisor) => Ok(TopDecl::Supervisor(self.parse_supervisor_decl()?)),
            Some(Token::KwSignal) => Ok(TopDecl::Signal(self.parse_signal_decl()?)),
            Some(Token::KwLet) | Some(Token::KwConst) => {
                Ok(TopDecl::Binding(self.parse_binding_decl()?))
            }
            Some(Token::KwShared) => Ok(TopDecl::Shared(self.parse_shared_decl()?)),
            Some(Token::KwUse) => Ok(TopDecl::Use(self.parse_use_decl()?)),
            _ => Err(format!(
                "unexpected {} at top level\n\
                 help: a file is made of declarations: node, signal, when, \
                 surge, cluster, enum, actor, domain, supervisor, shared, use",
                crate::lexer::describe_opt(self.peek())
            )),
        }
    }

    // ── Node ─────────────────────────────────────────────────────────────────

    fn parse_node_decl(&mut self) -> ParseResult<NodeDecl> {
        let start = self.span().start;
        self.advance(); // `node`
        let name = self.expect_ident()?;
        let generics = self.parse_generic_params()?;
        self.expect(&Token::LParen)?;
        let params = self.parse_params()?;
        self.expect(&Token::RParen)?;
        let ret = if self.eat(&Token::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let body = self.parse_block()?;
        let end = self.span().start;
        Ok(NodeDecl {
            span: start..end,
            name,
            generics,
            params,
            ret,
            body,
        })
    }

    // ── Surge ─────────────────────────────────────────────────────────────────

    fn parse_surge_decl(&mut self) -> ParseResult<SurgeDecl> {
        let start = self.span().start;
        self.advance(); // `surge`

        // Parse all bracketed attrs: [supervise: N, affinity: "domain"]
        let attrs = self.parse_surge_attrs()?;

        // kind: `on signal_ref` or `name`
        let kind = if self.eat(&Token::KwOn) {
            let signal = self.parse_signal_ref()?;
            SurgeKind::Reactive { signal }
        } else {
            let name = self.expect_ident()?;
            SurgeKind::Active { name }
        };

        self.expect(&Token::LParen)?;
        let params = self.parse_params()?;
        self.expect(&Token::RParen)?;
        let ret = if self.eat(&Token::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let body = self.parse_block()?;
        let end = self.span().start;
        Ok(SurgeDecl {
            span: start..end,
            kind,
            attrs,
            params,
            ret,
            body,
        })
    }

    fn parse_on_decl(&mut self) -> ParseResult<SurgeDecl> {
        let start = self.span().start;
        self.advance(); // `when` / `on`
        let signal = self.parse_signal_ref()?;
        self.expect(&Token::LParen)?;
        let params = self.parse_params()?;
        self.expect(&Token::RParen)?;
        let ret = if self.eat(&Token::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let body = self.parse_block()?;
        let end = self.span().start;
        Ok(SurgeDecl {
            span: start..end,
            kind: SurgeKind::Reactive { signal },
            attrs: SurgeAttrs::default(),
            params,
            ret,
            body,
        })
    }

    /// Parses optional `[supervise: N, affinity: "x"]` — any combination of surge attrs
    fn parse_surge_attrs(&mut self) -> ParseResult<SurgeAttrs> {
        let mut attrs = SurgeAttrs::default();
        if self.peek() != Some(&Token::LBracket) {
            return Ok(attrs);
        }

        // Peek at what's inside — if it's a surge attr keyword, parse it
        let is_surge_attr = matches!(
            self.peek2(),
            Some(Token::KwSupervise) | Some(Token::KwAffinity)
        );
        if !is_surge_attr {
            return Ok(attrs);
        }

        self.advance(); // [
        loop {
            match self.peek() {
                Some(Token::KwSupervise) => {
                    self.advance();
                    self.expect(&Token::Colon)?;
                    attrs.supervise = Some(if self.eat(&Token::KwInfinite) {
                        SuperviseCount::Infinite
                    } else {
                        match self.advance() {
                            Some(Token::LitInt(n)) => SuperviseCount::N(n as u32),
                            _ => {
                                return Err("expected integer or 'infinite' after supervise:".into())
                            }
                        }
                    });
                }
                Some(Token::KwAffinity) => {
                    self.advance();
                    self.expect(&Token::Colon)?;
                    attrs.affinity = Some(self.expect_string()?);
                }
                _ => break,
            }
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        self.expect(&Token::RBracket)?;
        Ok(attrs)
    }

    // ── Signal declaration ────────────────────────────────────────────────────

    fn parse_signal_decl(&mut self) -> ParseResult<SignalDecl> {
        let start = self.span().start;
        self.advance(); // `signal`
        let name = self.parse_signal_ref()?;
        self.expect(&Token::Colon)?;
        let ty = self.parse_type()?;
        let modes = self.parse_signal_modes()?;
        let end = self.span().start;
        Ok(SignalDecl {
            span: start..end,
            name,
            ty,
            modes,
        })
    }

    fn parse_signal_modes(&mut self) -> ParseResult<Vec<SignalMode>> {
        if self.peek() != Some(&Token::LBracket) {
            return Ok(vec![]);
        }
        self.advance(); // [
        let mut modes = Vec::new();
        loop {
            let mode = match self.peek() {
                Some(Token::KwBroadcast) => {
                    self.advance();
                    SignalMode::Broadcast
                }
                Some(Token::KwQueue) => {
                    self.advance();
                    SignalMode::Queue
                }
                Some(Token::KwLatest) => {
                    self.advance();
                    SignalMode::Latest
                }
                Some(Token::KwCoalesce) => {
                    self.advance();
                    SignalMode::Coalesce
                }
                Some(Token::KwRecursive) => {
                    self.advance();
                    SignalMode::Recursive
                }
                Some(Token::KwBuffer) => {
                    self.advance();
                    self.expect(&Token::Colon)?;
                    match self.advance() {
                        Some(Token::LitInt(n)) => SignalMode::Buffer(n as u64),
                        _ => return Err("expected integer after buffer:".into()),
                    }
                }
                Some(Token::KwSample) => {
                    self.advance();
                    self.expect(&Token::Colon)?;
                    match self.advance() {
                        Some(Token::LitInt(n)) => SignalMode::Sample(n as u64),
                        _ => return Err("expected integer (ms) after sample:".into()),
                    }
                }
                Some(Token::KwBudget) => {
                    self.advance();
                    self.expect(&Token::Colon)?;
                    match self.advance() {
                        Some(Token::LitInt(n)) => SignalMode::Budget(n as u64),
                        _ => return Err("expected integer (ms) after budget:".into()),
                    }
                }
                Some(Token::KwMaxDepth) => {
                    self.advance();
                    self.expect(&Token::Colon)?;
                    match self.advance() {
                        Some(Token::LitInt(n)) => SignalMode::MaxDepth(n as u32),
                        _ => return Err("expected integer after max_depth:".into()),
                    }
                }
                Some(Token::KwMaxFanout) => {
                    self.advance();
                    self.expect(&Token::Colon)?;
                    match self.advance() {
                        Some(Token::LitInt(n)) => SignalMode::MaxFanout(n as u32),
                        _ => return Err("expected integer after max_fanout:".into()),
                    }
                }
                Some(Token::KwScope) => {
                    self.advance();
                    self.expect(&Token::Colon)?;
                    let scope = match self.advance() {
                        Some(Token::KwLocal) => PropScope::Local,
                        Some(Token::KwCluster) => PropScope::Cluster,
                        _ => return Err("expected 'local' or 'cluster' after scope:".into()),
                    };
                    SignalMode::Scope(scope)
                }
                _ => return Err(format!("unknown signal mode: {:?}", self.peek())),
            };
            modes.push(mode);
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        self.expect(&Token::RBracket)?;
        Ok(modes)
    }

    fn parse_signal_ref(&mut self) -> ParseResult<SignalRef> {
        let name_or_domain = self.expect_ident()?;
        if self.eat(&Token::ColonColon) {
            let name = self.expect_ident()?;
            Ok(SignalRef {
                domain: Some(name_or_domain),
                name,
            })
        } else {
            Ok(SignalRef {
                domain: None,
                name: name_or_domain,
            })
        }
    }

    // ── Cluster ───────────────────────────────────────────────────────────────

    fn parse_cluster_decl(&mut self) -> ParseResult<ClusterDecl> {
        let start = self.span().start;
        self.advance(); // `cluster`
        let name = self.expect_ident()?;
        let generics = self.parse_generic_params()?;
        self.expect(&Token::LBrace)?;
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        while self.peek() != Some(&Token::RBrace) && !self.at_end() {
            match self.peek() {
                Some(Token::KwLet) | Some(Token::KwConst) => {
                    self.advance();
                    let fname = self.expect_ident()?;
                    self.expect(&Token::Colon)?;
                    let ty = self.parse_type()?;
                    self.eat(&Token::Comma);
                    fields.push(Field { name: fname, ty });
                }
                Some(Token::KwNode) => {
                    methods.push(self.parse_node_decl()?);
                }
                _ => {
                    return Err(format!(
                        "unexpected token in cluster body: {:?}",
                        self.peek()
                    ))
                }
            }
        }
        self.expect(&Token::RBrace)?;
        let end = self.span().start;
        Ok(ClusterDecl {
            span: start..end,
            name,
            generics,
            fields,
            methods,
        })
    }

    // ── Variant ───────────────────────────────────────────────────────────────

    fn parse_variant_decl(&mut self) -> ParseResult<VariantDecl> {
        let start = self.span().start;
        self.advance(); // `enum`
        let name = self.expect_ident()?;
        let generics = self.parse_generic_params()?;
        self.expect(&Token::LBrace)?;
        let mut cases = Vec::new();
        while self.peek() != Some(&Token::RBrace) && !self.at_end() {
            let case_name = self.expect_ident()?;
            let kind = match self.peek() {
                // tuple case: Name(T, U)
                Some(Token::LParen) => {
                    self.advance();
                    let mut types = Vec::new();
                    while self.peek() != Some(&Token::RParen) && !self.at_end() {
                        types.push(self.parse_type()?);
                        if !self.eat(&Token::Comma) {
                            break;
                        }
                    }
                    self.expect(&Token::RParen)?;
                    VariantCaseKind::Tuple(types)
                }
                // struct case: Name { field: T }
                Some(Token::LBrace) => {
                    self.advance();
                    let mut fields = Vec::new();
                    while self.peek() != Some(&Token::RBrace) && !self.at_end() {
                        let fname = self.expect_ident()?;
                        self.expect(&Token::Colon)?;
                        let ty = self.parse_type()?;
                        self.eat(&Token::Comma);
                        fields.push(Field { name: fname, ty });
                    }
                    self.expect(&Token::RBrace)?;
                    VariantCaseKind::Struct(fields)
                }
                _ => VariantCaseKind::Unit,
            };
            self.eat(&Token::Comma);
            cases.push(VariantCase {
                name: case_name,
                kind,
            });
        }
        self.expect(&Token::RBrace)?;
        let end = self.span().start;
        Ok(VariantDecl {
            span: start..end,
            name,
            generics,
            cases,
        })
    }

    // ── Actor ─────────────────────────────────────────────────────────────────

    fn parse_actor_decl(&mut self) -> ParseResult<ActorDecl> {
        let start = self.span().start;
        self.advance(); // `actor`
        let name = self.expect_ident()?;
        // Optional [pin: "worker"] before the body
        let attrs = self.parse_actor_pin_attr()?;
        self.expect(&Token::LBrace)?;
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        while self.peek() != Some(&Token::RBrace) && !self.at_end() {
            match self.peek() {
                Some(Token::KwLet) | Some(Token::KwConst) => {
                    // Actor fields: let name: Type = value
                    fields.push(self.parse_binding_decl()?);
                }
                Some(Token::KwNode) => {
                    methods.push(self.parse_node_decl()?);
                }
                _ => return Err(format!("unexpected token in actor body: {:?}", self.peek())),
            }
        }
        self.expect(&Token::RBrace)?;
        let end = self.span().start;
        Ok(ActorDecl {
            span: start..end,
            name,
            attrs,
            fields,
            methods,
        })
    }

    /// Parses optional `[pin: "worker_id"]` after actor name
    fn parse_actor_pin_attr(&mut self) -> ParseResult<ActorAttrs> {
        let mut attrs = ActorAttrs::default();
        if self.peek() != Some(&Token::LBracket) {
            return Ok(attrs);
        }
        if self.peek2() != Some(&Token::KwPin) {
            return Ok(attrs);
        }
        self.advance(); // [
        self.advance(); // pin
        self.expect(&Token::Colon)?;
        attrs.pin = Some(self.expect_string()?);
        self.expect(&Token::RBracket)?;
        Ok(attrs)
    }

    // ── Domain ────────────────────────────────────────────────────────────────

    fn parse_domain_decl(&mut self) -> ParseResult<DomainDecl> {
        let start = self.span().start;
        self.advance(); // `domain`
        let name = self.expect_ident()?;

        // Optional security attrs: domain payments [isolated, noncritical]
        let attrs = self.parse_domain_attrs()?;

        self.expect(&Token::LBrace)?;
        let mut signals = Vec::new();
        let mut surges = Vec::new();
        while self.peek() != Some(&Token::RBrace) && !self.at_end() {
            match self.peek() {
                Some(Token::KwSignal) => signals.push(self.parse_signal_decl()?),
                Some(Token::KwSurge)  => surges.push(self.parse_surge_decl()?),
                Some(Token::KwOn) | Some(Token::KwWhen) => surges.push(self.parse_on_decl()?),
                _ => return Err(format!("unexpected token in domain: {:?}", self.peek())),
            }
        }
        self.expect(&Token::RBrace)?;
        let end = self.span().start;
        Ok(DomainDecl {
            span: start..end,
            name,
            attrs,
            signals,
            surges,
        })
    }

    /// Parses optional `[isolated, noncritical, private, restricted]` after domain name
    fn parse_domain_attrs(&mut self) -> ParseResult<DomainAttrs> {
        let mut attrs = DomainAttrs::default();
        if self.peek() != Some(&Token::LBracket) {
            return Ok(attrs);
        }

        let is_domain_attr = matches!(
            self.peek2(),
            Some(Token::KwIsolated)
                | Some(Token::KwNoncritical)
                | Some(Token::KwPrivate)
                | Some(Token::KwRestricted)
        );
        if !is_domain_attr {
            return Ok(attrs);
        }

        self.advance(); // [
        loop {
            match self.peek() {
                Some(Token::KwIsolated) => {
                    self.advance();
                    attrs.isolated = true;
                }
                Some(Token::KwNoncritical) => {
                    self.advance();
                    attrs.noncritical = true;
                }
                Some(Token::KwPrivate) => {
                    self.advance();
                    attrs.private = true;
                }
                Some(Token::KwRestricted) => {
                    self.advance();
                    attrs.restricted = true;
                }
                _ => break,
            }
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        self.expect(&Token::RBracket)?;
        Ok(attrs)
    }

    // ── Supervisor ────────────────────────────────────────────────────────────

    fn parse_supervisor_decl(&mut self) -> ParseResult<SupervisorDecl> {
        let start = self.span().start;
        self.advance(); // `supervisor`
        let name = self.expect_ident()?;
        self.expect(&Token::LBrace)?;
        self.expect(&Token::KwStrategy)?;
        self.expect(&Token::Colon)?;
        let strategy = match self.advance() {
            Some(Token::KwOneForOne) => SupervisionStrategy::OneForOne,
            Some(Token::KwOneForAll) => SupervisionStrategy::OneForAll,
            Some(Token::KwRestForOne) => SupervisionStrategy::RestForOne,
            Some(Token::KwEscalate) => SupervisionStrategy::Escalate,
            other => return Err(format!("expected supervision strategy, got {:?}", other)),
        };
        let mut children = Vec::new();
        while self.peek() != Some(&Token::RBrace) && !self.at_end() {
            self.expect(&Token::KwChild)?;
            let label = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let callee = self.expect_ident()?;
            self.expect(&Token::LParen)?;
            let args = self.parse_args()?;
            self.expect(&Token::RParen)?;

            let (max_restarts, window_ms) = if self.eat(&Token::LBracket) {
                self.expect(&Token::KwMaxRestarts)?;
                self.expect(&Token::Colon)?;
                let mr = if self.eat(&Token::KwInfinite) {
                    SuperviseCount::Infinite
                } else {
                    match self.advance() {
                        Some(Token::LitInt(n)) => SuperviseCount::N(n as u32),
                        _ => return Err("expected integer for max_restarts".into()),
                    }
                };
                self.expect(&Token::Comma)?;
                self.expect(&Token::KwWindow)?;
                self.expect(&Token::Colon)?;
                let w = match self.advance() {
                    Some(Token::LitInt(n)) => n as u64,
                    _ => return Err("expected integer for window".into()),
                };
                self.expect(&Token::RBracket)?;
                (Some(mr), Some(w))
            } else {
                (None, None)
            };

            children.push(SupervisorChild {
                label,
                callee,
                args,
                max_restarts,
                window_ms,
            });
        }
        self.expect(&Token::RBrace)?;
        let end = self.span().start;
        Ok(SupervisorDecl {
            span: start..end,
            name,
            strategy,
            children,
        })
    }

    // ── Charge / Cortex / Link ────────────────────────────────────────────────

    fn parse_binding_decl(&mut self) -> ParseResult<BindingDecl> {
        let start = self.span().start;
        let immutable = match self.peek() {
            Some(Token::KwConst) => {
                self.advance();
                true
            }
            Some(Token::KwLet) => {
                self.advance();
                false
            }
            other => return Err(format!("expected `let` or `const`, got {:?}", other)),
        };
        let name = self.expect_ident()?;
        let ty = if self.eat(&Token::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(&Token::Eq)?;
        let value = self.parse_expr(0)?;
        let end = self.span().start;
        Ok(BindingDecl {
            span: start..end,
            name,
            immutable,
            ty,
            value,
        })
    }

    fn parse_shared_decl(&mut self) -> ParseResult<SharedDecl> {
        let start = self.span().start;
        self.advance(); // `shared`
        let name = self.expect_ident()?;
        self.expect(&Token::Colon)?;
        let ty = self.parse_type()?;
        self.expect(&Token::Eq)?;
        let value = self.parse_expr(0)?;
        let end = self.span().start;
        Ok(SharedDecl {
            span: start..end,
            name,
            ty,
            value,
        })
    }

    fn parse_use_decl(&mut self) -> ParseResult<UseDecl> {
        let start = self.span().start;
        self.advance(); // `use`
        let names = if self.eat(&Token::LBrace) {
            let mut ns = Vec::new();
            loop {
                ns.push(self.expect_ident()?);
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
            self.expect(&Token::RBrace)?;
            ns
        } else {
            vec![self.expect_ident()?]
        };
        self.expect(&Token::KwFrom)?;
        let source = self.expect_string()?;
        let end = self.span().start;
        Ok(UseDecl {
            span: start..end,
            names,
            source,
        })
    }

    // ── Block / Statements ────────────────────────────────────────────────────

    fn parse_block(&mut self) -> ParseResult<Vec<Stmt>> {
        self.expect(&Token::LBrace)?;
        let mut stmts = Vec::new();
        while self.peek() != Some(&Token::RBrace) && !self.at_end() {
            self.eat(&Token::Semi);
            if self.peek() == Some(&Token::RBrace) {
                break;
            }
            stmts.push(self.parse_stmt()?);
            self.eat(&Token::Semi);
        }
        self.expect(&Token::RBrace)?;
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> ParseResult<Stmt> {
        match self.peek() {
            Some(Token::KwBreak) => {
                let sp = self.span();
                self.advance();
                Ok(Stmt::Break(sp))
            }
            // Top-level declarations allowed inside blocks
            Some(Token::KwNode)
            | Some(Token::KwSurge)
            | Some(Token::KwOn)
            | Some(Token::KwCluster)
            | Some(Token::KwEnum)
            | Some(Token::KwActor)
            | Some(Token::KwLet)
            | Some(Token::KwConst)
            | Some(Token::KwUse) => Ok(Stmt::Decl(self.parse_top_decl()?)),
            // shared.field = expr -> expression; shared name: Type = val -> declaration
            Some(Token::KwShared) if self.peek2() != Some(&Token::Dot) => {
                Ok(Stmt::Decl(self.parse_top_decl()?))
            }
            // signal name: Type [modes] → declaration  (peek3 = Colon after name)
            // signal name(...)          → emit expression
            Some(Token::KwSignal) if matches!(self.peek3(), Some(Token::Colon)) => {
                Ok(Stmt::Decl(self.parse_top_decl()?))
            }
            _ => {
                let e = self.parse_expr(0)?;
                // trace send: expr <- expr
                if self.eat(&Token::TraceRecv) {
                    let span = e.span.clone();
                    let value = self.parse_expr(0)?;
                    let end = self.span().start;
                    Ok(Stmt::TraceSend {
                        channel: Box::new(e),
                        value: Box::new(value),
                        span: span.start..end,
                    })
                } else {
                    Ok(Stmt::Expr(e))
                }
            }
        }
    }

    // ── Params / Args ─────────────────────────────────────────────────────────

    fn parse_params(&mut self) -> ParseResult<Vec<Param>> {
        let mut params = Vec::new();
        while self.peek() != Some(&Token::RParen) && !self.at_end() {
            let name = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let ty = self.parse_type()?;
            params.push(Param { name, ty });
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        Ok(params)
    }

    fn parse_args(&mut self) -> ParseResult<Vec<Expr>> {
        let mut args = Vec::new();
        while self.peek() != Some(&Token::RParen) && !self.at_end() {
            args.push(self.parse_expr(0)?);
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        Ok(args)
    }

    fn parse_generic_params(&mut self) -> ParseResult<Vec<String>> {
        if self.peek() != Some(&Token::Lt) {
            return Ok(vec![]);
        }
        self.advance();
        let mut params = Vec::new();
        loop {
            params.push(self.expect_ident()?);
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        self.expect(&Token::Gt)?;
        Ok(params)
    }

    // ── Types ─────────────────────────────────────────────────────────────────

    fn parse_type(&mut self) -> ParseResult<TypeExpr> {
        let mut ty = self.parse_base_type()?;
        loop {
            match self.peek() {
                Some(Token::Question) => {
                    self.advance();
                    ty = TypeExpr::Optional(Box::new(ty));
                }
                Some(Token::Pipe) if self.peek2() == Some(&Token::KwError) => {
                    self.advance();
                    self.advance();
                    ty = TypeExpr::Result(Box::new(ty));
                }
                _ => break,
            }
        }
        Ok(ty)
    }

    fn parse_base_type(&mut self) -> ParseResult<TypeExpr> {
        match self.peek().cloned() {
            Some(Token::LBracket) => {
                self.advance();
                let inner = self.parse_type()?;
                if self.eat(&Token::Colon) {
                    let val = self.parse_type()?;
                    self.expect(&Token::RBracket)?;
                    Ok(TypeExpr::Map(Box::new(inner), Box::new(val)))
                } else {
                    self.expect(&Token::RBracket)?;
                    Ok(TypeExpr::List(Box::new(inner)))
                }
            }
            Some(Token::LParen) => {
                self.advance();
                let mut types = Vec::new();
                while self.peek() != Some(&Token::RParen) && !self.at_end() {
                    types.push(self.parse_type()?);
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                }
                self.expect(&Token::RParen)?;
                Ok(TypeExpr::Tuple(types))
            }
            Some(Token::TyTrace) => {
                self.advance();
                self.expect(&Token::Lt)?;
                let inner = self.parse_type()?;
                let cap = if self.eat(&Token::Comma) {
                    self.expect_ident()?; // "cap"
                    self.expect(&Token::Colon)?;
                    match self.advance() {
                        Some(Token::LitInt(n)) => Some(n as u64),
                        _ => return Err("expected integer for cap".into()),
                    }
                } else {
                    None
                };
                self.expect(&Token::Gt)?;
                Ok(TypeExpr::Trace {
                    inner: Box::new(inner),
                    cap,
                })
            }
            Some(Token::KwNode) => {
                self.advance();
                self.expect(&Token::LParen)?;
                let mut ptypes = Vec::new();
                while self.peek() != Some(&Token::RParen) && !self.at_end() {
                    ptypes.push(self.parse_type()?);
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                }
                self.expect(&Token::RParen)?;
                self.expect(&Token::Arrow)?;
                let ret = self.parse_type()?;
                Ok(TypeExpr::NodeType {
                    params: ptypes,
                    ret: Box::new(ret),
                })
            }
            Some(tok) => {
                let name = match tok {
                    Token::Ident(n) => {
                        self.advance();
                        n
                    }
                    Token::TyInt => {
                        self.advance();
                        "Int".into()
                    }
                    Token::TyInt32 => {
                        self.advance();
                        "Int32".into()
                    }
                    Token::TyInt8 => {
                        self.advance();
                        "Int8".into()
                    }
                    Token::TyFloat => {
                        self.advance();
                        "Float".into()
                    }
                    Token::TyFloat32 => {
                        self.advance();
                        "Float32".into()
                    }
                    Token::TyBool => {
                        self.advance();
                        "Bool".into()
                    }
                    Token::TyStr => {
                        self.advance();
                        "Str".into()
                    }
                    Token::TyByte => {
                        self.advance();
                        "Byte".into()
                    }
                    Token::TyNull => {
                        self.advance();
                        "Null".into()
                    }
                    Token::KwError => {
                        self.advance();
                        "error".into()
                    }
                    _ => return Err(format!("expected type, got {:?}", tok)),
                };
                // generic args: Name<T, U>
                if self.peek() == Some(&Token::Lt) {
                    self.advance();
                    let mut args = Vec::new();
                    while self.peek() != Some(&Token::Gt) && !self.at_end() {
                        args.push(self.parse_type()?);
                        if !self.eat(&Token::Comma) {
                            break;
                        }
                    }
                    self.expect(&Token::Gt)?;
                    Ok(TypeExpr::Generic(name, args))
                } else {
                    Ok(TypeExpr::Named(name))
                }
            }
            None => Err("unexpected end of input in type".into()),
        }
    }

    // ── Expressions (Pratt parser) ────────────────────────────────────────────

    fn parse_expr(&mut self, min_bp: u8) -> ParseResult<Expr> {
        let start = self.span().start;
        let mut left = self.parse_prefix()?;
        // Tight postfix (field access, calls, indexing, try) binds harder than
        // any binary operator: `shared.n > 0` must parse as `(shared.n) > 0`.
        left = self.parse_postfix_tight(left, start)?;

        loop {
            // Infix binary operators
            let op = match self.peek() {
                Some(Token::Plus) => BinOp::Add,
                Some(Token::Minus) => BinOp::Sub,
                Some(Token::Star) => BinOp::Mul,
                Some(Token::Slash) => BinOp::Div,
                Some(Token::Percent) => BinOp::Mod,
                Some(Token::StarStar) => BinOp::Pow,
                Some(Token::EqEq) => BinOp::Eq,
                Some(Token::BangEq) => BinOp::Ne,
                Some(Token::Lt) => BinOp::Lt,
                Some(Token::LtEq) => BinOp::Le,
                Some(Token::Gt) => BinOp::Gt,
                Some(Token::GtEq) => BinOp::Ge,
                Some(Token::KwAnd) => BinOp::And,
                Some(Token::KwOr) => BinOp::Or,
                Some(Token::Amp) => BinOp::BitAnd,
                Some(Token::Pipe) => BinOp::BitOr,
                Some(Token::Tilde) => BinOp::BitXor,
                Some(Token::LtLt) => BinOp::Shl,
                Some(Token::GtGt) => BinOp::Shr,
                _ => break,
            };
            let (lbp, rbp) = infix_bp(op);
            if lbp < min_bp {
                break;
            }
            self.advance();
            let right = self.parse_expr(rbp)?;
            let end = self.span().start;
            left = Expr {
                span: start..end,
                kind: ExprKind::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
            };
        }

        // Loose tail operators: pipe, coalesce, assignment
        loop {
            match self.peek().cloned() {
                Some(Token::DoubleQuestion) => {
                    self.advance();
                    let right = self.parse_expr(0)?;
                    let end = self.span().start;
                    left = Expr {
                        span: start..end,
                        kind: ExprKind::Coalesce(Box::new(left), Box::new(right)),
                    };
                }
                Some(Token::PipeArrow) => {
                    self.advance();
                    let name = self.expect_ident()?;
                    let end = self.span().start;
                    left = Expr {
                        span: start..end,
                        kind: ExprKind::Pipe(Box::new(left), name),
                    };
                }
                _ => {
                    if let Some(aop) = self.peek_assign_op() {
                        self.advance();
                        let val = self.parse_expr(0)?;
                        let end = self.span().start;
                        left = Expr {
                            span: start..end,
                            kind: ExprKind::Assign {
                                target: Box::new(left),
                                op: aop,
                                value: Box::new(val),
                            },
                        };
                    } else {
                        break;
                    }
                }
            }
        }

        Ok(left)
    }

    fn parse_postfix_tight(&mut self, mut left: Expr, start: usize) -> ParseResult<Expr> {
        loop {
            match self.peek().cloned() {
                Some(Token::Dot) => {
                    self.advance();
                    let name = self.expect_ident()?;
                    let end = self.span().start;
                    if self.eat(&Token::LParen) {
                        let args = self.parse_args()?;
                        self.expect(&Token::RParen)?;
                        let end2 = self.span().start;
                        left = Expr {
                            span: start..end2,
                            kind: ExprKind::Method {
                                object: Box::new(left),
                                name,
                                args,
                            },
                        };
                    } else if let Some(aop) = self.peek_assign_op() {
                        self.advance();
                        let val = self.parse_expr(0)?;
                        let end2 = self.span().start;
                        let target = Expr {
                            span: start..end,
                            kind: ExprKind::Field {
                                object: Box::new(left),
                                name,
                            },
                        };
                        left = Expr {
                            span: start..end2,
                            kind: ExprKind::Assign {
                                target: Box::new(target),
                                op: aop,
                                value: Box::new(val),
                            },
                        };
                    } else if self.peek() == Some(&Token::LBrace) {
                        // Variant struct literal: Variant.Case { field: value }
                        // Only if left is a simple capitalized Ident (the variant
                        // type name) — `shared.n {` must NOT eat a block brace.
                        let is_variant_name = matches!(
                            &left.kind,
                            ExprKind::Ident(n) if n.chars().next().is_some_and(|c| c.is_uppercase())
                        );
                        if !is_variant_name {
                            left = Expr {
                                span: start..end,
                                kind: ExprKind::Field { object: Box::new(left), name },
                            };
                            continue;
                        }
                        if let ExprKind::Ident(ref variant_name) = left.kind.clone() {
                            self.advance(); // {
                            let mut fields = Vec::new();
                            while self.peek() != Some(&Token::RBrace) && !self.at_end() {
                                let fname = self.expect_ident()?;
                                self.expect(&Token::Colon)?;
                                let fval = self.parse_expr(0)?;
                                fields.push((fname, fval));
                                self.eat(&Token::Comma);
                            }
                            self.expect(&Token::RBrace)?;
                            let end2 = self.span().start;
                            left = Expr {
                                span: start..end2,
                                kind: ExprKind::VariantLit {
                                    variant: variant_name.clone(),
                                    case: name,
                                    payload: VariantPayload::Struct(fields),
                                },
                            };
                        } else {
                            left = Expr {
                                span: start..end,
                                kind: ExprKind::Field {
                                    object: Box::new(left),
                                    name,
                                },
                            };
                        }
                    } else {
                        left = Expr {
                            span: start..end,
                            kind: ExprKind::Field {
                                object: Box::new(left),
                                name,
                            },
                        };
                    }
                }
                Some(Token::LParen) => {
                    self.advance();
                    let args = self.parse_args()?;
                    self.expect(&Token::RParen)?;
                    let end = self.span().start;
                    left = Expr {
                        span: start..end,
                        kind: ExprKind::Call {
                            callee: Box::new(left),
                            args,
                        },
                    };
                }
                Some(Token::LBracket) => {
                    self.advance();
                    let idx = self.parse_expr(0)?;
                    self.expect(&Token::RBracket)?;
                    let end = self.span().start;
                    left = Expr {
                        span: start..end,
                        kind: ExprKind::Index {
                            object: Box::new(left),
                            index: Box::new(idx),
                        },
                    };
                }
                Some(Token::Question) => {
                    self.advance();
                    let end = self.span().start;
                    left = Expr {
                        span: start..end,
                        kind: ExprKind::Try(Box::new(left)),
                    };
                }
                _ => break,
            }
        }

        Ok(left)
    }

    fn peek_assign_op(&self) -> Option<AssignOp> {
        match self.peek() {
            Some(Token::Eq) => Some(AssignOp::Assign),
            Some(Token::PlusEq) => Some(AssignOp::AddAssign),
            Some(Token::MinusEq) => Some(AssignOp::SubAssign),
            Some(Token::StarEq) => Some(AssignOp::MulAssign),
            Some(Token::SlashEq) => Some(AssignOp::DivAssign),
            Some(Token::StarStarEq) => Some(AssignOp::PowAssign),
            _ => None,
        }
    }

    fn parse_prefix(&mut self) -> ParseResult<Expr> {
        let start = self.span().start;

        match self.peek().cloned() {
            // ── Literals ──────────────────────────────────────────────────────
            Some(Token::LitInt(n))
            | Some(Token::LitHexInt(n))
            | Some(Token::LitBinInt(n))
            | Some(Token::LitOctInt(n)) => {
                self.advance();
                Ok(Expr {
                    span: start..self.span().start,
                    kind: ExprKind::Int(n),
                })
            }
            Some(Token::LitFloat(f)) => {
                self.advance();
                Ok(Expr {
                    span: start..self.span().start,
                    kind: ExprKind::Float(f),
                })
            }
            Some(Token::LitString(s)) => {
                self.advance();
                Ok(Expr {
                    span: start..self.span().start,
                    kind: ExprKind::Str(s),
                })
            }
            Some(Token::KwTrue) => {
                self.advance();
                Ok(Expr {
                    span: start..self.span().start,
                    kind: ExprKind::Bool(true),
                })
            }
            Some(Token::KwFalse) => {
                self.advance();
                Ok(Expr {
                    span: start..self.span().start,
                    kind: ExprKind::Bool(false),
                })
            }
            Some(Token::KwNull) => {
                self.advance();
                Ok(Expr {
                    span: start..self.span().start,
                    kind: ExprKind::Null,
                })
            }
            Some(Token::KwSelf) => {
                self.advance();
                Ok(Expr {
                    span: start..self.span().start,
                    kind: ExprKind::SelfKw,
                })
            }
            // shared.field access
            Some(Token::KwShared) => {
                self.advance();
                Ok(Expr {
                    span: start..self.span().start,
                    kind: ExprKind::Ident("shared".to_string()),
                })
            }
            // error { message: "...", code: N } creates an error value
            Some(Token::KwError) => {
                self.advance();
                self.expect(&Token::LBrace)?;
                let mut fields = Vec::new();
                while self.peek() != Some(&Token::RBrace) && !self.at_end() {
                    let fname = self.expect_ident()?;
                    self.expect(&Token::Colon)?;
                    let fval = self.parse_expr(0)?;
                    fields.push((fname, fval));
                    self.eat(&Token::Comma);
                }
                self.expect(&Token::RBrace)?;
                Ok(Expr {
                    span: start..self.span().start,
                    kind: ExprKind::ClusterLit {
                        name: "error".into(),
                        fields,
                    },
                })
            }

            // ── Identifier / cluster literal / domain ref ────────────────────
            Some(Token::Ident(name)) => {
                self.advance();
                if self.eat(&Token::ColonColon) {
                    // domain::name
                    let member = self.expect_ident()?;
                    Ok(Expr {
                        span: start..self.span().start,
                        kind: ExprKind::DomainRef {
                            domain: name,
                            name: member,
                        },
                    })
                } else if self.peek() == Some(&Token::LBrace)
                    && name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false)
                    && self
                        .peek2()
                        .map(|t| matches!(t, Token::Ident(_)))
                        .unwrap_or(false)
                    && self
                        .tokens
                        .get(self.pos + 2)
                        .map(|t| t.node == Token::Colon)
                        .unwrap_or(false)
                {
                    // Cluster literal: MyCluster { field: value, ... }
                    // Heuristic: uppercase Ident + { Ident : = field list
                    self.advance(); // {
                    let mut fields = Vec::new();
                    while self.peek() != Some(&Token::RBrace) && !self.at_end() {
                        let fname = self.expect_ident()?;
                        self.expect(&Token::Colon)?;
                        let fval = self.parse_expr(0)?;
                        fields.push((fname, fval));
                        self.eat(&Token::Comma);
                    }
                    self.expect(&Token::RBrace)?;
                    Ok(Expr {
                        span: start..self.span().start,
                        kind: ExprKind::ClusterLit { name, fields },
                    })
                } else {
                    Ok(Expr {
                        span: start..self.span().start,
                        kind: ExprKind::Ident(name),
                    })
                }
            }

            // ── List and Map literals ─────────────────────────────────────────
            // [1, 2, 3]           → ListLit
            // ["key": "val", ...]  → MapLit
            Some(Token::LBracket) => {
                self.advance();
                // Empty collection: []
                if self.eat(&Token::RBracket) {
                    return Ok(Expr {
                        span: start..self.span().start,
                        kind: ExprKind::ListLit(vec![]),
                    });
                }
                // Empty map: [:]
                if self.peek() == Some(&Token::Colon) {
                    self.advance();
                    self.expect(&Token::RBracket)?;
                    return Ok(Expr {
                        span: start..self.span().start,
                        kind: ExprKind::MapLit(vec![]),
                    });
                }

                let first = self.parse_expr(0)?;

                if self.eat(&Token::Colon) {
                    // Map literal: [key: val, ...]
                    let val = self.parse_expr(0)?;
                    let mut pairs = vec![(first, val)];
                    while self.eat(&Token::Comma) {
                        if self.peek() == Some(&Token::RBracket) { break; }
                        let k = self.parse_expr(0)?;
                        self.expect(&Token::Colon)?;
                        let v = self.parse_expr(0)?;
                        pairs.push((k, v));
                    }
                    self.eat(&Token::Comma);
                    self.expect(&Token::RBracket)?;
                    Ok(Expr {
                        span: start..self.span().start,
                        kind: ExprKind::MapLit(pairs),
                    })
                } else {
                    // List literal: [e1, e2, ...]
                    let mut items = vec![first];
                    while self.eat(&Token::Comma) {
                        if self.peek() == Some(&Token::RBracket) { break; }
                        items.push(self.parse_expr(0)?);
                    }
                    self.eat(&Token::Comma);
                    self.expect(&Token::RBracket)?;
                    Ok(Expr {
                        span: start..self.span().start,
                        kind: ExprKind::ListLit(items),
                    })
                }
            }

            // ── Grouped ───────────────────────────────────────────────────────
            Some(Token::LParen) => {
                self.advance();
                let e = self.parse_expr(0)?;
                self.expect(&Token::RParen)?;
                Ok(e)
            }

            // ── Unary ─────────────────────────────────────────────────────────
            Some(Token::Minus) => {
                self.advance();
                let operand = self.parse_expr(80)?;
                let end = self.span().start;
                Ok(Expr {
                    span: start..end,
                    kind: ExprKind::Unary {
                        op: UnOp::Neg,
                        operand: Box::new(operand),
                    },
                })
            }
            Some(Token::KwNot) => {
                self.advance();
                let operand = self.parse_expr(80)?;
                let end = self.span().start;
                Ok(Expr {
                    span: start..end,
                    kind: ExprKind::Unary {
                        op: UnOp::Not,
                        operand: Box::new(operand),
                    },
                })
            }

            // ── Pulse / Yield ─────────────────────────────────────────────────
            Some(Token::KwPulse) => {
                self.advance();
                let v = self.parse_expr(0)?;
                let end = self.span().start;
                Ok(Expr {
                    span: start..end,
                    kind: ExprKind::Pulse(Box::new(v)),
                })
            }
            Some(Token::KwYield) => {
                self.advance();
                let v = self.parse_expr(0)?;
                let end = self.span().start;
                Ok(Expr {
                    span: start..end,
                    kind: ExprKind::Yield(Box::new(v)),
                })
            }

            // ── Signal emit ───────────────────────────────────────────────────
            // Syntax: signal [domain::]name [priority]? (ownership? args)
            // Example: signal auth::login[critical](share data)
            Some(Token::KwSignal) => {
                self.advance();
                let name = self.parse_signal_ref()?;

                // optional emit priority: signal name[critical](...)
                let priority = if self.eat(&Token::LBracket) {
                    let p = match self.peek() {
                        Some(Token::KwCritical) => {
                            self.advance();
                            SignalPriority::Critical
                        }
                        Some(Token::KwRealtime) => {
                            self.advance();
                            SignalPriority::Realtime
                        }
                        Some(Token::KwIdle) => {
                            self.advance();
                            SignalPriority::Idle
                        }
                        _ => SignalPriority::Normal,
                    };
                    self.expect(&Token::RBracket)?;
                    p
                } else {
                    SignalPriority::Normal
                };

                self.expect(&Token::LParen)?;

                // optional ownership prefix: share / move
                let ownership = match self.peek() {
                    Some(Token::KwShare) => {
                        self.advance();
                        SignalOwnership::Share
                    }
                    Some(Token::KwMove) => {
                        self.advance();
                        SignalOwnership::Move
                    }
                    _ => SignalOwnership::Clone,
                };

                let args = self.parse_args()?;
                self.expect(&Token::RParen)?;
                let end = self.span().start;
                Ok(Expr {
                    span: start..end,
                    kind: ExprKind::Signal {
                        name,
                        args,
                        ownership,
                        priority,
                    },
                })
            }

            // ── Open ──────────────────────────────────────────────────────────
            Some(Token::KwStart) => {
                self.advance();
                // start "label" callee(...) OR start Supervisor
                let label = match self.peek() {
                    Some(Token::LitString(_)) => {
                        if let Some(Token::LitString(s)) = self.advance() {
                            Some(s)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                let name = self.expect_ident()?;
                // Could be `start Supervisor` (no parens) or `start surge(args)`
                if self.peek() != Some(&Token::LParen) {
                    let end = self.span().start;
                    return Ok(Expr {
                        span: start..end,
                        kind: ExprKind::StartSupervisor(name),
                    });
                }
                self.expect(&Token::LParen)?;
                let args = self.parse_args()?;
                self.expect(&Token::RParen)?;
                let end = self.span().start;
                let callee = Box::new(Expr {
                    span: start..end,
                    kind: ExprKind::Ident(name),
                });
                Ok(Expr {
                    span: start..end,
                    kind: ExprKind::Start {
                        label,
                        callee,
                        args,
                    },
                })
            }

            // ── Close ─────────────────────────────────────────────────────────
            Some(Token::KwStop) => {
                self.advance();
                let label = self.expect_string()?;
                let end = self.span().start;
                Ok(Expr {
                    span: start..end,
                    kind: ExprKind::Stop(label),
                })
            }

            // ── Collect ───────────────────────────────────────────────────────
            Some(Token::KwAwait) => {
                self.advance();
                let handle = self.parse_expr(0)?;
                let end = self.span().start;
                Ok(Expr {
                    span: start..end,
                    kind: ExprKind::Await(Box::new(handle)),
                })
            }

            // ── Rest ──────────────────────────────────────────────────────────
            Some(Token::KwSleep) => {
                self.advance();
                let ms = self.parse_expr(0)?;
                let end = self.span().start;
                Ok(Expr {
                    span: start..end,
                    kind: ExprKind::Sleep(Box::new(ms)),
                })
            }

            // ── Trace receive ─────────────────────────────────────────────────
            Some(Token::TraceRecv) => {
                self.advance();
                let ch = self.parse_expr(0)?;
                let end = self.span().start;
                Ok(Expr {
                    span: start..end,
                    kind: ExprKind::TraceRecv(Box::new(ch)),
                })
            }

            // ── When ──────────────────────────────────────────────────────────
            Some(Token::KwIf) => self.parse_if_expr(),

            // ── Loop ──────────────────────────────────────────────────────────
            Some(Token::KwLoop) => self.parse_loop_expr(),

            // ── Match ─────────────────────────────────────────────────────────
            Some(Token::KwMatch) => self.parse_match_expr(),

            // ── Break ─────────────────────────────────────────────────────────
            Some(Token::KwBreak) => {
                self.advance();
                Ok(Expr {
                    span: start..self.span().start,
                    kind: ExprKind::Break,
                })
            }

            Some(Token::LBrace) => Err(
                "unexpected '{' — an expression cannot start with a brace\n\
                 help: map literals use brackets: [\"key\": value]; \
                 an empty map is [:]".to_string()
            ),
            other => Err(format!(
                "unexpected {} — expected an expression here",
                crate::lexer::describe_opt(other.as_ref())
            )),
        }
    }

    // ── Control flow expressions ──────────────────────────────────────────────

    fn parse_if_expr(&mut self) -> ParseResult<Expr> {
        let start = self.span().start;
        self.advance(); // `if`
        let cond = self.parse_expr(0)?;
        let then = self.parse_block()?;
        let mut else_ifs = Vec::new();
        let mut else_ = None;
        loop {
            if self.peek() == Some(&Token::KwElse) && self.peek2() == Some(&Token::KwIf) {
                self.advance();
                self.advance();
                let ec = self.parse_expr(0)?;
                let eb = self.parse_block()?;
                else_ifs.push((ec, eb));
            } else if self.eat(&Token::KwElse) {
                else_ = Some(self.parse_block()?);
                break;
            } else {
                break;
            }
        }
        let end = self.span().start;
        Ok(Expr {
            span: start..end,
            kind: ExprKind::If {
                cond: Box::new(cond),
                then,
                else_ifs,
                else_,
            },
        })
    }

    fn parse_loop_expr(&mut self) -> ParseResult<Expr> {
        let start = self.span().start;
        self.advance(); // `loop`
        let binding = if self.peek() != Some(&Token::LBrace) {
            let var = self.expect_ident()?;
            self.expect(&Token::KwIn)?;
            let iter = self.parse_expr(0)?;
            Some((var, Box::new(iter)))
        } else {
            None
        };
        let body = self.parse_block()?;
        let end = self.span().start;
        Ok(Expr {
            span: start..end,
            kind: ExprKind::Loop { binding, body },
        })
    }

    fn parse_match_expr(&mut self) -> ParseResult<Expr> {
        let start = self.span().start;
        self.advance(); // `match`

        // Select form: `match { binding <- ch -> ... }`
        if self.peek() == Some(&Token::LBrace) {
            if self.peek2() == Some(&Token::Ident(String::new())) || {
                // check if the first token inside is an ident followed by <-
                self.tokens
                    .get(self.pos + 1)
                    .map(|t| matches!(t.node, Token::Ident(_)))
                    .unwrap_or(false)
                    && self
                        .tokens
                        .get(self.pos + 2)
                        .map(|t| t.node == Token::TraceRecv)
                        .unwrap_or(false)
            } {
                self.advance(); // {
                let mut arms = Vec::new();
                while self.peek() != Some(&Token::RBrace) && !self.at_end() {
                    let binding = self.expect_ident()?;
                    self.expect(&Token::TraceRecv)?;
                    let channel = self.parse_expr(0)?;
                    self.expect(&Token::Arrow)?;
                    let body = if self.peek() == Some(&Token::LBrace) {
                        self.parse_block()?
                    } else {
                        vec![Stmt::Expr(self.parse_expr(0)?)]
                    };
                    self.eat(&Token::Comma);
                    arms.push(SelectArm {
                        binding,
                        channel,
                        body,
                    });
                }
                self.expect(&Token::RBrace)?;
                let end = self.span().start;
                return Ok(Expr {
                    span: start..end,
                    kind: ExprKind::Select { arms },
                });
            }
        }

        let subject = self.parse_expr(0)?;
        self.expect(&Token::LBrace)?;
        let mut arms = Vec::new();
        while self.peek() != Some(&Token::RBrace) && !self.at_end() {
            let pattern = self.parse_pattern()?;
            self.expect(&Token::Arrow)?;
            let body = if self.peek() == Some(&Token::LBrace) {
                self.parse_block()?
            } else {
                vec![Stmt::Expr(self.parse_expr(0)?)]
            };
            self.eat(&Token::Comma);
            arms.push(MatchArm { pattern, body });
        }
        self.expect(&Token::RBrace)?;
        let end = self.span().start;
        Ok(Expr {
            span: start..end,
            kind: ExprKind::Match {
                subject: Box::new(subject),
                arms,
            },
        })
    }

    fn parse_pattern(&mut self) -> ParseResult<Pattern> {
        match self.peek().cloned() {
            Some(Token::Ident(n)) if n == "_" => {
                self.advance();
                Ok(Pattern::Wildcard)
            }
            Some(Token::LitInt(n)) => {
                self.advance();
                if matches!(self.peek(), Some(Token::Range) | Some(Token::RangeInc)) {
                    let inclusive = self.peek() == Some(&Token::RangeInc);
                    self.advance();
                    match self.advance() {
                        Some(Token::LitInt(end)) => Ok(Pattern::Range {
                            start: n,
                            end,
                            inclusive,
                        }),
                        _ => Err("expected integer in range pattern".into()),
                    }
                } else {
                    Ok(Pattern::Literal(LitPattern::Int(n)))
                }
            }
            Some(Token::LitFloat(f)) => {
                self.advance();
                Ok(Pattern::Literal(LitPattern::Float(f)))
            }
            Some(Token::LitString(s)) => {
                self.advance();
                Ok(Pattern::Literal(LitPattern::Str(s)))
            }
            Some(Token::KwTrue) => {
                self.advance();
                Ok(Pattern::Literal(LitPattern::Bool(true)))
            }
            Some(Token::KwFalse) => {
                self.advance();
                Ok(Pattern::Literal(LitPattern::Bool(false)))
            }
            Some(Token::KwNull) => {
                self.advance();
                Ok(Pattern::Literal(LitPattern::Null))
            }
            Some(Token::Ident(name)) => {
                self.advance();
                // Variant pattern: Name.Case { fields } or Name.Case(bindings)
                if self.eat(&Token::Dot) {
                    let case = self.expect_ident()?;
                    let bindings = match self.peek() {
                        Some(Token::LParen) => {
                            self.advance();
                            let mut bs = Vec::new();
                            while self.peek() != Some(&Token::RParen) && !self.at_end() {
                                bs.push(self.expect_ident()?);
                                if !self.eat(&Token::Comma) {
                                    break;
                                }
                            }
                            self.expect(&Token::RParen)?;
                            VariantBindings::Tuple(bs)
                        }
                        Some(Token::LBrace) => {
                            self.advance();
                            let mut bs = Vec::new();
                            while self.peek() != Some(&Token::RBrace) && !self.at_end() {
                                bs.push(self.expect_ident()?);
                                self.eat(&Token::Comma);
                            }
                            self.expect(&Token::RBrace)?;
                            VariantBindings::Struct(bs)
                        }
                        _ => VariantBindings::None,
                    };
                    Ok(Pattern::Variant {
                        variant: name,
                        case,
                        bindings,
                    })
                } else {
                    let ty = if self.eat(&Token::Colon) {
                        Some(self.parse_type()?)
                    } else {
                        None
                    };
                    Ok(Pattern::Bind { name, ty })
                }
            }
            other => Err(format!("expected pattern, got {:?}", other)),
        }
    }
}

// ── Pratt binding powers ──────────────────────────────────────────────────────

fn infix_bp(op: BinOp) -> (u8, u8) {
    match op {
        BinOp::Or => (10, 11),
        BinOp::And => (20, 21),
        BinOp::BitOr => (25, 26),
        BinOp::BitXor => (27, 28),
        BinOp::BitAnd => (29, 30),
        BinOp::Eq | BinOp::Ne => (30, 31),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => (40, 41),
        BinOp::Shl | BinOp::Shr => (45, 46),
        BinOp::Add | BinOp::Sub => (50, 51),
        BinOp::Mul | BinOp::Div | BinOp::Mod => (60, 61),
        BinOp::Pow => (70, 69), // right-assoc
    }
}
