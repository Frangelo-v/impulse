# Impulse

Impulse is a dormant reactive propagation language: programs declare signals, dormant handlers, actors, domains, and supervised long-running surges. Work wakes through propagation instead of unstructured detached tasks.

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
