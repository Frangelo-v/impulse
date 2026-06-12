# ⚡ Impulse

**The language where waiting costs nothing.**

[![Release](https://img.shields.io/github/v/release/Frangelo-v/impulse?include_prereleases&label=release)](https://github.com/Frangelo-v/impulse/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)](compiler/)

Impulse is a *dormant reactive* language: your code declares typed signals and
handlers that **sleep at literally 0% CPU** until a signal wakes them — like
neurons waiting for an impulse. No event loop to write, no callbacks to nest,
no polling anywhere in the runtime.

```impulse
signal pedido: Str [broadcast]
signal enviado: Str [broadcast]

// Dormant handler: costs nothing until a signal arrives
when pedido(producto: Str) {
    io.print("cobrando " + producto)
    signal enviado(producto)
}

when enviado(producto: Str) {
    io.print("en camino: " + producto)
}

node main() {
    signal pedido("teclado")      // wakes the whole chain
}
```

Crashes don't take the program down — supervisors restart failed work,
Erlang-style:

```impulse
supervisor App {
    strategy: one_for_one
    child worker: handler() [max_restarts: 5, window: 60_000]
}
```

**Measured, not promised** (see [the benchmark](#the-thesis-measured)):
an idle Impulse program uses **0% CPU** while a polling equivalent burns ~98%
of a core, and the interpreter propagates **~630,000 signals/sec**.

Programs declare signals, dormant handlers, actors, domains, and supervised
long-running surges. Work wakes through propagation instead of unstructured
detached tasks.

## A note from the author

Impulse is a very ambitious personal project I dreamed up a long time ago.
I wanted to build something people would *truly love to use*: a language that
works like a brain — code that sleeps and only wakes when something real
happens, easy to write, honest about its costs.

I'm one person, so not everything I imagined made it into this version —
the full vision (native compilation, distributed propagation, the complete
runtime described in the spec) is still ahead. But the core thesis is here,
working and measured, and I'd rather share it now than keep it in a drawer:
I just wanted to see what happens when it meets the world.

This is the first beta. I'll keep publishing versions as time goes on.
I hope you like it — and if you try it, tell me what you think. Every issue,
idea, or "this broke" report genuinely helps.

— Frangelo ([@Frangelo-v](https://github.com/Frangelo-v))

## Current Components

- `compiler/` - lexer, parser, semantic analysis, execution backends, and runtime primitives.
- `examples/` - runnable Impulse programs. Each `NN_*.imp` with a matching `.expected` file is a golden test: `compiler/tests/golden.rs` runs it and compares the exact output on every `cargo test`.
- `spec/` - language specification.
- `docs/` - architecture notes and design direction.
- `vscode-impulse/` - VS Code syntax, diagnostics, snippets, and commands.

## Standard Library (built-in namespaces)

- `io` — `print(v)`, `eprint(v)`
- `math` — `floor`, `ceil`, `sqrt`, `abs`, `min`, `max`, `pow`
- `time` — `now()` (epoch milliseconds)
- `fs` — `read(path)`, `write(path, content)`, `append(path, content)`, `exists(path)`, `delete(path)`
- `http` — `get(url)`, `post(url, body)` (HTTPS, 30s timeout, JSON content type on post)

`fs` and `http` return `Str | error`, so failures match with `e: error` like any
user error. String literals support `\n`, `\t`, `\r`, `\"`, `\\` escapes.

Collections mutate in place: `list.push(v)` / `list.pop()` and
`map.set(k, v)` / `map.delete(k)` modify the variable they are called on
(calling them on a temporary value is an error). `await <- "channel"` parks
the thread on a condition variable — no polling, per spec guarantee #6.

## The Thesis, Measured

Impulse's core claim is *zero cost while dormant*: handlers sleep until a signal
wakes them, so an idle program consumes no CPU. `bench\bench.ps1` proves it on
this machine (release build, 10s idle measurement, June 2026):

| Waiting style                  | CPU while idle |
|--------------------------------|----------------|
| Impulse signals (`when` + sleeping surge) | **0%** |
| Busy-poll loop (same language, same work) | **~89% of a core** |

Signal propagation throughput: **~600,000 signals/sec** through the tree-walk
interpreter (100,000 signals, `bench\throughput.imp`).

```powershell
powershell -File bench\bench.ps1
```

## Run The Example

```powershell
C:\dev\impulse-target\debug\impulsec.exe examples\hello.imp
```

Runtime activation counters are available with:

```powershell
C:\dev\impulse-target\debug\impulsec.exe examples\hello.imp --runtime-stats
```

## Build

```powershell
$env:PATH = "C:\msys64\mingw64\bin;$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo +stable-x86_64-pc-windows-gnu build -p impulsec
```

## Test

```powershell
$env:PATH = "C:\msys64\mingw64\bin;$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo +stable-x86_64-pc-windows-gnu test -p impulsec
```

Note: Windows Smart App Control may block freshly built test binaries
("Una directiva de Control de aplicaciones bloqueó este archivo").
Rebuilding after any source change produces a new binary that usually passes.
