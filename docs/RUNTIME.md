# Runtime Design Notes

The production Impulse runtime is activation-based rather than call-stack-based.

## Activation Record

Every unit of work should eventually be represented as:

```text
Activation {
  id
  trace_id
  signal
  payload
  target_handler
  domain
  priority
  depth
  fanout
  deadline
  supervisor
}
```

This gives the runtime enough information to batch, schedule, observe, cancel, and supervise work.

## Dormancy

The runtime is dormant when:

- all domain queues are empty
- global queues are empty
- actor mailboxes are empty
- no timer is due
- no IO source is ready
- no supervisor restart is pending

When dormant, worker threads park on OS primitives. There should be no spin loops.

## Safety Controls

- propagation depth budget
- fanout budget
- wall/CPU budget
- queue capacity
- per-signal delivery mode
- cycle detection
- supervisor restart windows
- actor mailbox pressure

## Observability

The runtime should emit structured events for:

- signal emitted
- handler scheduled
- handler started
- handler completed
- propagation budget exceeded
- actor mailbox pressure
- domain queue depth
- worker parked/woken
- surge crashed/restarted/dead

The development executor already reports activation counters:

```powershell
impulsec examples\hello.imp --runtime-stats
```

Current counters:

- signals emitted
- activations enqueued
- activations completed
- maximum activation queue depth
