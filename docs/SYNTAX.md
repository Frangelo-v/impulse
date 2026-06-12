# Impulse Syntax Guide

Impulse should be readable before it is clever.

## Preferred Surface

```impulse
signal login: Str [broadcast]

when login(name: Str) {
    io.print("hello " + name)
}

node main() {
    signal login("alex")
}
```

## Core Words

| Word | Meaning |
|---|---|
| `signal` | A typed stimulus |
| `when` | Dormant handler for a signal |
| `node` | Callable computation |
| `pulse` | Return a value |
| `surge` | Long-running supervised worker |
| `actor` | Isolated mutable state |
| `domain` | Propagation boundary |
| `shared` | Atomic scalar module state |

## Compatibility

`on signal(...)` is accepted as an alias for `when signal(...)`, but examples and documentation should use `when`.
