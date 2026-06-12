# Impulse Architecture

Impulse has one core idea: dormant reactive propagation. A program declares what can wake, how it propagates, and which ownership boundary contains it. The runtime decides where and when to execute activations.

## Layers

1. **Frontend**
   - Lexer
   - Parser
   - AST
   - Diagnostics

2. **Analysis**
   - Signal declaration validation
   - Signal graph construction
   - Cycle detection
   - Dead signal warnings
   - Propagation budget checks
   - Domain visibility checks

3. **Execution**
   - `execution::treewalk` is the development executor.
   - It now dispatches signals through a runtime activation queue instead of recursive direct calls.
   - It runs examples and gives fast feedback while the production backend evolves.

4. **Runtime**
   - Signal registry
   - Dormant scheduler
   - Timer wheel
   - Domain-local queues
   - Actor mailboxes
   - Supervisors
   - Metrics and traces

## Runtime Direction

The final runtime should avoid a single global queue as the primary model. The target model is:

- each domain owns local activation queues
- signal emissions become activation records
- activations carry propagation context, budget, trace id, priority, and domain
- work stealing happens only across compatible domains and only after imbalance
- timer and IO wait sets park workers instead of polling
- supervisors own every active surge and restart policy
- metrics are emitted as first-class runtime events

## Syntax Direction

Beginner-facing code should prefer:

```impulse
signal player_joined: Player [broadcast]

when player_joined(player: Player) {
    notify_friends(player)
}
```

`when` reads naturally for reactive handlers. `on` remains accepted as an alias for compatibility.
