# Impulse Language Specification
**Version:** 0.4.0 architecture draft

Impulse is a dormant reactive propagation language. Programs are not built around a call stack that happens to spawn async work; they are built around typed signals that activate only the regions that need to wake.

The core identity is:

- computation starts from stimuli
- signals propagate through declared paths
- inactive regions sleep at zero CPU cost
- concurrency is structured and observable
- no surge runs detached from ownership, supervision, or a propagation root

Impulse should feel simple to write:

```impulse
signal player_joined: Player [broadcast]

when player_joined(player: Player) {
    notify_friends(player)
}
```

The runtime decides scheduling, wakeups, batching, locality, and backpressure.

---

## 1. Architectural Findings

The previous surface syntax had too much private vocabulary. `charge`, `refrac`, `cortex`, `collect`, `rest`, `open`, `close`, `variant`, and `static` made simple code look harder than the execution model really is. These names did not add architectural power; they added translation cost.

The stronger model keeps the neural identity where it matters:

| Kept | Reason |
|---|---|
| `signal` | The central activation primitive |
| `pulse` | A node emits its result |
| `surge` | A long-lived concurrent unit |
| `node` | A computation node in the graph |
| `actor` | A standard isolation concept |
| `domain` | Ownership and propagation boundary |

The simplified model replaces obscure terms:

| Old | New |
|---|---|
| `surge on event(...)` | `when event(...)` |
| `charge` | `let` |
| `charge refrac` | `const` |
| `cortex` | `shared` |
| `when` / `or when` | `if` / `else if` |
| `rest` | `sleep` |
| `collect` | `await` |
| `static` errors | `error` |
| `link` | `use` |
| `open` / `close` | `start` / `stop` |
| `variant` | `enum` |

---

## 2. Mental Model

Impulse has four user-facing runtime concepts:

| Concept | Meaning |
|---|---|
| `signal` | A typed stimulus that activates work |
| `when` | A dormant reactive handler for a signal |
| `surge` | An explicitly started long-running unit |
| `actor` | Serialized isolated mutable state |

Everything else exists to protect those concepts: domains bound propagation, supervisors bound failures, and the compiler validates the signal graph before runtime.

---

## 3. Keywords

```text
node      signal    when      on        surge     pulse
let       const     shared    actor     domain
supervisor strategy child     start     stop
await     sleep     enum      cluster   match
if        else      loop      break     in
use       from      true      false     null
self      error     yield     share     move
and       or        not       is        as
```

---

## 4. Bindings

```impulse
let count: Int = 0
let name = "impulse"

const MAX_USERS: Int = 100_000
```

`let` creates a mutable binding. `const` creates an immutable binding. Type inference is allowed where the initializer is unambiguous.

---

## 5. Nodes

```impulse
node add(a: Int, b: Int) -> Int {
    pulse a + b
}
```

`pulse` terminates the current node or surge and emits its result to the caller or awaiting parent.

---

## 6. Types

```impulse
cluster User {
    let id: Int
    let name: Str
}

enum Status {
    Online
    Away { since: Int }
    Offline
}
```

`cluster` is a named record type. `enum` is a sum type and is matched exhaustively.

Errors are explicit result types:

```impulse
node parse_id(s: Str) -> Int | error {
    if s == "" {
        pulse error { message: "empty id", code: 400 }
    }
    pulse 42
}
```

There are no exceptions and no stack unwinding.

---

## 7. Control Flow

```impulse
if x > 10 {
    io.print("big")
} else if x == 0 {
    io.print("zero")
} else {
    io.print("small")
}

loop item in items {
    process(item)
}

match status {
    Status.Online -> "online"
    Status.Away { since } -> "away"
    Status.Offline -> "offline"
}
```

---

## 8. Signals

Signals must be declared before use.

```impulse
signal login: LoginData [broadcast, max_fanout: 64, budget: 10]
signal mouse: Position [latest]
signal jobs: Job [queue]
signal logs: LogEntry [buffer: 4096]
```

Delivery modes:

| Mode | Behavior |
|---|---|
| `broadcast` | All listeners activate concurrently |
| `queue` | One listener consumes each emission |
| `latest` | Keep only the newest pending value |
| `buffer: N` | Ring buffer with bounded retention |
| `sample: N` | Coalesce emissions within a time window |
| `recursive` | Allows intentional cycles |
| `budget: N` | Propagation CPU budget in milliseconds |
| `max_depth: N` | Maximum propagation depth |
| `max_fanout: N` | Maximum activations from one signal |
| `scope: local|cluster` | Local or distributed-ready propagation |

Emitting:

```impulse
signal login(data)
signal login(share data)
signal login(move data)
signal login[critical](data)
```

---

## 9. Reactive Handlers

`when` declares a dormant handler. It is registered once at startup and wakes only when its signal fires. `on` remains accepted as a compatibility alias, but new code should prefer `when`.

```impulse
when login(data: LoginData) {
    SessionStore.add(data.user)
    signal audit::login(data)
}
```

Reactive handlers may only appear at module top level or inside a `domain`. This prevents accidental listener multiplication inside loops or active surges.

---

## 10. Active Surges

`surge` declares explicitly started long-running work.

```impulse
surge server(port: Int) {
    let listener = http.listen(port)
    loop req in listener {
        signal web::request(share req)
    }
}

node main() {
    start "web" server(8080)
}
```

Lifecycle operations:

```impulse
let handle = start compute(100)
let result = await handle
stop "web"
sleep 5_000
```

`sleep` is dormant suspension, not polling.

---

## 11. Actors And Shared Scalars

Use `actor` for mutable compound state:

```impulse
actor Sessions {
    let users: [Int: User] = [:]

    node add(user: User) {
        self.users[user.id] = user
    }
}
```

Use `shared` only for atomic scalar module state:

```impulse
shared hit_count: Int = 0

when web::request(req: Request) {
    shared.hit_count += 1
}
```

Compound shared state belongs in actors, not `shared`.

---

## 12. Domains

Domains are propagation and ownership boundaries.

```impulse
domain payments [isolated, restricted] {
    signal charge_card: Payment [broadcast, budget: 5]

    when charge_card(payment: Payment) {
        authorize(payment)
    }
}
```

Domain attributes:

| Attribute | Meaning |
|---|---|
| `isolated` | Crashes cannot propagate out |
| `noncritical` | Domain failure does not stop the runtime |
| `private` | Signals are not visible externally |
| `restricted` | External emission requires explicit `use` permission |

---

## 13. Supervision

```impulse
supervisor App {
    strategy: one_for_one
    child server: server(8080) [max_restarts: 10, window: 60_000]
}

node main() {
    start App
}
```

Strategies:

| Strategy | Behavior |
|---|---|
| `one_for_one` | Restart only the crashed child |
| `one_for_all` | Restart all children |
| `rest_for_one` | Restart the crashed child and later children |
| `escalate` | Propagate failure to parent |

---

## 14. Runtime Architecture

The scalable runtime should be organized as:

1. Signal ingress normalizes emissions into activation records.
2. The signal graph maps each signal to dormant handler regions.
3. Per-domain schedulers enqueue activations with locality hints.
4. Work stealing occurs only after imbalance thresholds are crossed.
5. Timer wheels park sleeping surges without polling.
6. Supervisors own all active surges and failure propagation.
7. The dormant monitor parks workers when all queues, timers, and IO wait sets are empty.

The runtime must track:

- idle CPU usage
- wake latency
- unnecessary wakeups
- signal amplification ratio
- queue depth by signal and domain
- handler heat
- crash storms
- propagation budget violations

---

## 15. Observability

Reactive systems are not viable without first-class tooling. The compiler/runtime must emit:

- signal graph
- propagation trace
- scheduler timeline
- actor heatmap
- dead signal warnings
- amplification analysis
- cycle diagnostics
- crash and restart trace

Compiler flags:

```text
impulsec app.imp --emit-tokens
impulsec app.imp --emit-ast
impulsec app.imp --emit-graph
impulsec app.imp --check
```

---

## 16. Grammar Snapshot

```ebnf
program      = top_decl* EOF

top_decl     = node_decl | when_decl | surge_decl | cluster_decl | enum_decl
             | actor_decl | domain_decl | supervisor_decl
             | signal_decl | let_decl | shared_decl | use_decl

node_decl    = "node" IDENT "(" params? ")" ("->" type)? block
when_decl    = ("when" | "on") domain_signal "(" params? ")" ("->" type)? block
surge_decl   = "surge" surge_opts? IDENT "(" params? ")" ("->" type)? block

signal_decl  = "signal" domain_signal ":" type signal_modes?
let_decl     = ("let" | "const") IDENT (":" type)? "=" expr
shared_decl  = "shared" IDENT ":" type "=" expr
use_decl     = "use" link_names "from" STRING

cluster_decl = "cluster" IDENT "{" (field | node_decl)* "}"
enum_decl    = "enum" IDENT "{" enum_case* "}"
actor_decl   = "actor" IDENT "{" (let_decl | node_decl)* "}"

expr         = literal | IDENT | call | signal_emit | pulse | start | stop
             | await | sleep | if_expr | loop_expr | match_expr

if_expr      = "if" expr block ("else" "if" expr block)* ("else" block)?
signal_emit  = "signal" domain_signal priority? "(" ownership? args? ")"
```

---

## 17. Guarantees

Impulse aims to guarantee:

1. No undeclared signals.
2. No accidental reactive cycles.
3. No detached concurrent work.
4. No unbounded propagation without explicit budgets or recursion.
5. No compound shared mutable state outside actors.
6. No polling-based sleep.
7. No invisible signal graph.

Impulse remains:

**dormant reactive propagation computing.**
