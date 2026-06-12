#[cfg(test)]
mod tests {
    use crate::lexer::tokenize;
    use crate::parser::parse;
    use crate::semantic::analyze;
    use crate::execution;

    fn check(src: &str) -> (bool, Vec<String>, Vec<String>) {
        let tokens = tokenize(src);
        let program = match parse(tokens, src) {
            Ok(p) => p,
            Err(e) => return (false, vec![format!("PARSE: {}", e)], vec![]),
        };
        let result = analyze(&program);
        let errors = result.errors.iter().map(|e| e.message.clone()).collect();
        let warnings = result.warnings.iter().map(|w| w.message.clone()).collect();
        (result.errors.is_empty(), errors, warnings)
    }

    /// Parse, analyze, and execute a program. Returns Ok(()) on success or Err(msg).
    fn exec(src: &str) -> Result<(), String> {
        let tokens = tokenize(src);
        let program = parse(tokens, src).map_err(|e| format!("PARSE: {}", e))?;
        let result = analyze(&program);
        if !result.errors.is_empty() {
            return Err(result.errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; "));
        }
        execution::run_with_options(&program, execution::RunOptions::default())
    }

    // ── Lexer ─────────────────────────────────────────────────────────────────

    #[test]
    fn lex_signal_with_modes() {
        let src = r#"signal user_login: Str [broadcast, max_fanout: 64, budget: 10]"#;
        let tokens = tokenize(src);
        assert!(!tokens.is_empty());
        assert!(tokens
            .iter()
            .any(|t| format!("{:?}", t.node).contains("KwSignal")));
        assert!(tokens
            .iter()
            .any(|t| format!("{:?}", t.node).contains("KwBroadcast")));
        assert!(tokens
            .iter()
            .any(|t| format!("{:?}", t.node).contains("KwMaxFanout")));
        assert!(tokens
            .iter()
            .any(|t| format!("{:?}", t.node).contains("KwBudget")));
    }

    #[test]
    fn lex_domain_with_attrs() {
        let src = r#"domain payments [isolated, noncritical] { }"#;
        let tokens = tokenize(src);
        assert!(tokens
            .iter()
            .any(|t| format!("{:?}", t.node).contains("KwDomain")));
        assert!(tokens
            .iter()
            .any(|t| format!("{:?}", t.node).contains("KwIsolated")));
        assert!(tokens
            .iter()
            .any(|t| format!("{:?}", t.node).contains("KwNoncritical")));
    }

    #[test]
    fn lex_surge_with_affinity() {
        let src = r#"surge[supervise: 5, affinity: "physics"] worker() { }"#;
        let tokens = tokenize(src);
        assert!(tokens
            .iter()
            .any(|t| format!("{:?}", t.node).contains("KwSurge")));
        assert!(tokens
            .iter()
            .any(|t| format!("{:?}", t.node).contains("KwSupervise")));
        assert!(tokens
            .iter()
            .any(|t| format!("{:?}", t.node).contains("KwAffinity")));
    }

    #[test]
    fn lex_all_signal_modes() {
        let src = r#"signal a: Int [broadcast] signal b: Int [queue] signal c: Int [latest] signal d: Int [buffer: 512] signal e: Int [sample: 16] signal f: Int [recursive]"#;
        let tokens = tokenize(src);
        for kw in &[
            "KwBroadcast",
            "KwQueue",
            "KwLatest",
            "KwBuffer",
            "KwSample",
            "KwRecursive",
        ] {
            assert!(
                tokens.iter().any(|t| format!("{:?}", t.node).contains(kw)),
                "missing {}",
                kw
            );
        }
    }

    // ── Parser ────────────────────────────────────────────────────────────────

    #[test]
    fn parse_minimal_program() {
        let src = r#"
            signal ready: Int [broadcast]
            node main() { }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_signal_declaration() {
        let src = r#"
            signal user_login: Str [broadcast, max_fanout: 100, budget: 20]
            node main() { }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_reactive_when_handler() {
        let src = r#"
            signal login: Str [broadcast]
            when login(name: Str) { }
            node main() { }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_reactive_on_alias() {
        let src = r#"
            signal login: Str [broadcast]
            on login(name: Str) { }
            node main() { }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_active_surge_with_supervise() {
        let src = r#"
            signal tick: Int [broadcast]
            surge[supervise: 5] worker() {
                sleep 1000
            }
            node main() { start worker() }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_actor() {
        let src = r#"
            actor Counter {
                let value: Int = 0
                node inc() { self.value += 1 }
                node get() -> Int { pulse self.value }
            }
            node main() { Counter.inc() }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_enum_and_match() {
        let src = r#"
            enum Color { Red  Green  Blue }
            node name(c: Color) -> Str {
                match c {
                    Color.Red   -> "red"
                    Color.Green -> "green"
                    Color.Blue  -> "blue"
                }
            }
            node main() { }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_cluster_literal() {
        let src = r#"
            cluster Point { let x: Int  let y: Int }
            node main() {
                let p = Point { x: 1, y: 2 }
            }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_shared_declaration_and_access() {
        let src = r#"
            shared count: Int = 0
            signal tick: Int [broadcast]
            when tick(n: Int) {
                shared.count += 1
            }
            node main() { }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_domain_with_attrs() {
        let src = r#"
            domain payments [isolated] {
                signal charge_card: Int [broadcast, budget: 5]
            }
            node main() { }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_supervisor() {
        let src = r#"
            surge worker() { sleep 1000 }
            supervisor App {
                strategy: one_for_one
                child w: worker() [max_restarts: 5, window: 60000]
            }
            node main() { start App }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_signal_emit_with_priority() {
        let src = r#"
            signal alert: Str [broadcast]
            node main() {
                signal alert("hi")
                signal alert[critical]("urgent")
            }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_error_handling() {
        let src = r#"
            node parse(s: Str) -> Int | error {
                if s == "" {
                    pulse error { message: "empty", code: 400 }
                }
                pulse 42
            }
            node main() {
                match parse("abc") {
                    n: Int    -> n
                    e: error -> 0
                }
            }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_pipe_operator() {
        let src = r#"
            node double(n: Int) -> Int { pulse n * 2 }
            node main() {
                let result = 5 |> double
            }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    // ── Semantic ──────────────────────────────────────────────────────────────

    #[test]
    fn semantic_undeclared_signal_error() {
        let src = r#"
            node main() {
                signal ghost("hello")
            }
        "#;
        let (ok, errs, _) = check(src);
        assert!(!ok, "should have failed");
        assert!(
            errs.iter().any(|e| e.contains("not declared")),
            "expected undeclared signal error, got: {:?}",
            errs
        );
    }

    #[test]
    fn semantic_no_main_error() {
        let src = r#"
            signal ping: Int [broadcast]
        "#;
        let (ok, errs, _) = check(src);
        assert!(!ok);
        assert!(
            errs.iter().any(|e| e.contains("main")),
            "expected no-main error, got: {:?}",
            errs
        );
    }

    #[test]
    fn semantic_dead_signal_warning() {
        let src = r#"
            signal orphan: Int [broadcast]
            node main() { }
        "#;
        let (ok, _, warnings) = check(src);
        assert!(ok, "should have no errors");
        assert!(
            warnings.iter().any(|w| w.contains("no registered")),
            "expected dead signal warning, got: {:?}",
            warnings
        );
    }

    #[test]
    fn semantic_shared_non_scalar_error() {
        let src = r#"
            cluster Config { let x: Int }
            shared cfg: Config = Config { x: 1 }
            node main() { }
        "#;
        let (ok, errs, _) = check(src);
        assert!(!ok, "should have failed — shared must be scalar");
        assert!(
            errs.iter().any(|e| e.contains("scalar")),
            "expected scalar type error, got: {:?}",
            errs
        );
    }

    #[test]
    fn semantic_budget_zero_error() {
        let src = r#"
            signal bad: Int [budget: 0]
            node main() { }
        "#;
        let (_, _, warnings) = check(src);
        assert!(
            warnings.iter().any(|w| w.contains("budget: 0ms")),
            "expected budget=0 warning, got: {:?}",
            warnings
        );
    }

    // ── Execution ─────────────────────────────────────────────────────────────

    #[test]
    fn exec_node_call_and_pulse() {
        let src = r#"
            node add(a: Int, b: Int) -> Int { pulse a + b }
            node main() {
                let r = add(3, 4)
            }
        "#;
        assert!(exec(src).is_ok(), "node call failed");
    }

    #[test]
    fn exec_all_binops() {
        let src = r#"
            node main() {
                let a = 10 + 3
                let b = 10 - 3
                let c = 10 * 3
                let d = 10 / 3
                let e = 10 % 3
                let f = 2 ** 8
                let g = 5 < 10
                let h = 10 <= 10
                let i = 10 > 5
                let j = 10 >= 10
                let k = 10 == 10
                let l = 10 != 9
                let m = true and false
                let n = false or true
                let o = 0b1010 & 0b1100
                let p = 0b1010 | 0b1100
                let q = 0b1010 ^ 0b1100
                let r = 1 << 4
                let s = 16 >> 2
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_if_else_chain() {
        let src = r#"
            node classify(n: Int) -> Str {
                if n > 100 { pulse "big" }
                else if n > 10 { pulse "medium" }
                else { pulse "small" }
            }
            node main() {
                let a = classify(200)
                let b = classify(50)
                let c = classify(3)
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_loop_with_break() {
        let src = r#"
            node main() {
                let i = 0
                loop {
                    i += 1
                    if i >= 5 { break }
                }
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_loop_over_list() {
        let src = r#"
            node main() {
                let items = [1, 2, 3, 4, 5]
                let sum = 0
                loop item in items {
                    sum += item
                }
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_list_operations() {
        let src = r#"
            node main() {
                let nums = [10, 20, 30]
                let first = nums[0]
                let len   = nums.len()
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_map_operations() {
        let src = r#"
            node main() {
                let m = ["name": "impulse", "version": "0.1"]
                let v = m["name"]
                let k = m.has("version")
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_match_variant() {
        let src = r#"
            enum Color { Red  Green  Blue }
            node name(c: Color) -> Str {
                match c {
                    Color.Red   -> "red"
                    Color.Green -> "green"
                    Color.Blue  -> "blue"
                }
            }
            node main() {
                let r = name(Color.Red)
                let g = name(Color.Green)
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_match_range_pattern() {
        let src = r#"
            node grade(score: Int) -> Str {
                match score {
                    90..=100 -> "A"
                    70..=89  -> "B"
                    50..=69  -> "C"
                    _        -> "F"
                }
            }
            node main() {
                let a = grade(95)
                let b = grade(75)
                let c = grade(30)
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_match_struct_variant() {
        let src = r#"
            enum Shape {
                Circle { radius: Int }
                Rect   { width: Int, height: Int }
            }
            node area(s: Shape) -> Int {
                match s {
                    Shape.Circle { radius } -> radius * radius
                    Shape.Rect   { width, height } -> width * height
                }
            }
            node main() {
                let c = area(Shape.Circle { radius: 5 })
                let r = area(Shape.Rect { width: 3, height: 4 })
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_error_propagation() {
        let src = r#"
            node parse(s: Str) -> Int | error {
                if s == "" {
                    pulse error { message: "empty", code: 400 }
                }
                pulse 42
            }
            node main() {
                match parse("abc") {
                    n: Int   -> n
                    e: error -> 0
                }
                match parse("") {
                    n: Int   -> n
                    e: error -> 0
                }
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_pulse_inside_if_exits_node() {
        // Regresión: `pulse` dentro de un `if` debe salir del nodo entero.
        // Si no corta, se ejecuta `a / b` con b == 0 y exec devuelve Err.
        let src = r#"
            node divide(a: Int, b: Int) -> Int | error {
                if b == 0 {
                    pulse error { message: "div0", code: 500 }
                }
                pulse a / b
            }
            node main() {
                let r = divide(7, 0)
            }
        "#;
        assert!(exec(src).is_ok(), "pulse en guard no cortó la ejecución del nodo");
    }

    #[test]
    fn exec_pulse_inside_loop_exits_node() {
        let src = r#"
            node primero_mayor(limite: Int) -> Int {
                loop n in [1, 5, 10, 20] {
                    if n > limite {
                        pulse n
                    }
                }
                pulse 0 - 1
            }
            node main() {
                let r = primero_mayor(7)
                if r == 10 { } else { let boom = 1 / 0 }
            }
        "#;
        assert!(exec(src).is_ok(), "pulse dentro de loop no devolvió el valor correcto");
    }

    #[test]
    fn exec_unit_variant_match() {
        // Regresión: `Estado.Online` sin payload debe construir la variante
        // y matchear su patrón (antes evaluaba a null).
        let src = r#"
            enum Estado {
                Online
                Offline
            }
            node describe(e: Estado) -> Str {
                match e {
                    Estado.Online  -> "on"
                    Estado.Offline -> "off"
                }
            }
            node main() {
                let a = describe(Estado.Online)
                let b = describe(Estado.Offline)
                if a == "on" and b == "off" { } else { let boom = 1 / 0 }
            }
        "#;
        assert!(exec(src).is_ok(), "variante unitaria no matcheó su patrón");
    }

    #[test]
    fn parse_field_access_then_binary_op() {
        // Regresión: el postfijo (.campo) debe ligar más fuerte que cualquier
        // operador binario, y `shared.n {` no debe comerse la llave del bloque.
        let src = r#"
            shared n: Int = 0
            cluster P { let x: Int }
            node main() {
                if shared.n > 0 { }
                if shared.n { }
                let p = P { x: 3 }
                if p.x == 3 and shared.n < 10 { }
                let suma = p.x + shared.n * 2
            }
        "#;
        let (ok, errs, _) = check(src);
        assert!(ok, "errors: {:?}", errs);
    }

    #[test]
    fn parse_errors_are_human_readable() {
        // Los mensajes deben hablar el idioma del usuario, no el del compilador:
        // nada de nombres internos de tokens (LBrace, Gt, Some(...)).
        let src = r#"node main() { let m = { "a": 1 } }"#;
        let (ok, errs, _) = check(src);
        assert!(!ok);
        let msg = &errs[0];
        assert!(msg.contains("help:"), "sin pista: {:?}", msg);
        assert!(!msg.contains("LBrace") && !msg.contains("Some("), "token interno filtrado: {:?}", msg);
    }

    #[test]
    fn lex_string_escapes_resolved() {
        use crate::lexer::Token;
        let tokens = tokenize(r#""a\nb\tc\\d\"e""#);
        let Token::LitString(s) = &tokens[0].node else { panic!("no es string") };
        assert_eq!(s, "a\nb\tc\\d\"e");
    }

    #[test]
    fn exec_collections_mutate_for_real() {
        // Regresión: push/pop/set/delete deben mutar la variable, no un clon.
        let src = r#"
            node main() {
                let lista = [1, 2]
                lista.push(3)
                let ultimo = lista.pop()
                if lista.len() == 2 and ultimo == 3 { } else { let boom = 1 / 0 }

                let mapa = ["a": 1]
                mapa.set("b", 2)
                mapa.delete("a")
                if mapa.len() == 1 and mapa.has("b") { } else { let boom = 1 / 0 }
            }
        "#;
        assert!(exec(src).is_ok(), "las colecciones no mutaron de verdad");
    }

    #[test]
    fn exec_mutating_method_on_temporary_errors() {
        // Mutar un temporal sería un no-op silencioso — debe ser error honesto.
        let src = r#"
            node main() {
                [1, 2].push(3)
            }
        "#;
        let r = exec(src);
        assert!(r.is_err(), "push sobre temporal debería fallar");
        assert!(r.unwrap_err().contains("temporary"), "mensaje poco claro");
    }

    #[test]
    fn exec_trace_channels_no_polling() {
        // El canal entrega valores ya encolados; con Condvar no hay espera activa.
        let src = r#"
            node main() {
                "canal" <- 42
                "canal" <- 7
                let a = await <- "canal"
                let b = await <- "canal"
                if a == 42 and b == 7 { } else { let boom = 1 / 0 }
            }
        "#;
        assert!(exec(src).is_ok(), "canales rotos tras quitar el polling");
    }

    #[test]
    fn exec_json_parse_telegram_shape() {
        // La forma exacta que devuelve getUpdates de Telegram, navegada con .get()
        let src = r#"
            node main() {
                let body = "{\"ok\": true, \"result\": [{\"update_id\": 99, \"message\": {\"text\": \"hola\", \"chat\": {\"id\": 555}}}]}"
                let data = json.parse(body)
                let updates = data.get("result")
                let upd = updates.get(0)
                let uid = upd.get("update_id")
                let msg = upd.get("message")
                let text = msg.get("text")
                let chat_id = msg.get("chat").get("id")
                if uid == 99 and text == "hola" and chat_id == 555 { } else { let boom = 1 / 0 }
            }
        "#;
        assert!(exec(src).is_ok(), "no se navegó la respuesta de Telegram");
    }

    #[test]
    fn exec_json_roundtrip() {
        let src = r#"
            node main() {
                let original = ["nombre": "impulse", "version": 1]
                let texto = json.stringify(original)
                let vuelta = json.parse(texto)
                if vuelta.get("nombre") == "impulse" and vuelta.get("version") == 1 { } else { let boom = 1 / 0 }
            }
        "#;
        assert!(exec(src).is_ok(), "json stringify/parse no es ida y vuelta");
    }

    #[test]
    fn exec_http_encode() {
        let src = r#"
            node main() {
                let e = http.encode("a b&c")
                if e == "a%20b%26c" { } else { let boom = 1 / 0 }
            }
        "#;
        assert!(exec(src).is_ok(), "http.encode incorrecto");
    }

    #[test]
    fn exec_pipe_operator() {
        let src = r#"
            node double(n: Int) -> Int { pulse n * 2 }
            node add_one(n: Int) -> Int { pulse n + 1 }
            node main() {
                let r = 5 |> double
                let s = 5 |> add_one
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_actor_state_mutation() {
        let src = r#"
            actor Counter {
                let value: Int = 0
                node inc() { self.value += 1 }
                node get() -> Int { pulse self.value }
            }
            node main() {
                Counter.inc()
                Counter.inc()
                Counter.inc()
                let v = Counter.get()
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_shared_cortex() {
        let src = r#"
            shared count: Int = 0
            signal tick: Int [broadcast]
            when tick(n: Int) {
                shared.count += 1
            }
            node main() {
                signal tick(1)
                signal tick(1)
                signal tick(1)
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_reactive_signal_chain() {
        let src = r#"
            signal ping: Str [broadcast]
            signal pong: Str [broadcast]
            when ping(msg: Str) {
                signal pong("pong:" + msg)
            }
            when pong(msg: Str) {
                let _ = msg
            }
            node main() {
                signal ping("hello")
                signal ping("world")
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_cluster_method() {
        let src = r#"
            cluster Point {
                let x: Int
                let y: Int
                node dist_sq() -> Int {
                    pulse self.x * self.x + self.y * self.y
                }
            }
            node main() {
                let p = Point { x: 3, y: 4 }
                let d = p.dist_sq()
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_string_methods() {
        let src = r#"
            node main() {
                let s   = "  Hello World  "
                let t   = s.trim()
                let u   = t.to_upper()
                let l   = t.to_lower()
                let has = t.contains("World")
                let len = t.len()
                let parts = "a,b,c".split(",")
                let rep = "abc".replace("b", "X")
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_domain_signal() {
        let src = r#"
            domain analytics [noncritical] {
                signal view: Str [broadcast]
                when view(path: Str) {
                    let _ = path
                }
            }
            node main() {
                signal analytics::view("/home")
                signal analytics::view("/about")
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_coalesce_operator() {
        let src = r#"
            node maybe_null(flag: Bool) -> Int | error {
                if flag { pulse 42 }
                else    { pulse error { message: "none", code: 0 } }
            }
            node main() {
                let a = maybe_null(true)
                let b = maybe_null(false) ?? 0
            }
        "#;
        assert!(exec(src).is_ok());
    }

    #[test]
    fn exec_math_builtins() {
        let src = r#"
            node main() {
                let a = math.floor(3.7)
                let b = math.ceil(3.2)
                let c = math.sqrt(16.0)
                let d = math.abs(-5)
                let e = math.min(3.0, 7.0)
                let f = math.max(3.0, 7.0)
            }
        "#;
        assert!(exec(src).is_ok());
    }
}
