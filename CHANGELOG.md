# Changelog

## 0.1.0-beta.1 — 2026-06-12

Primera versión beta pública del lenguaje Impulse.

### Lenguaje
- Señales tipadas con modos de entrega (`broadcast`, `queue`, `latest`, `buffer`,
  `budget`, `max_fanout`, `max_depth`, `recursive`) y prioridades.
- Handlers durmientes `when`, surges activos supervisados, actores con estado
  serializado, `shared` atómico, dominios con aislamiento.
- Clusters (records con métodos), enums con payload y `match` exhaustivo con
  bindings, rangos y patrones de tipo.
- Errores como valores (`Int | error`), sin excepciones; `??` y truthiness
  reconocen errores.
- Pipe `|>`, canales con `<-` / `await`, `select` sobre canales.
- Escapes de string (`\n`, `\t`, `\r`, `\"`, `\\`).
- Colecciones con mutación real: `push`/`pop`/`set`/`delete` modifican la
  variable; mutar un temporal es un error en runtime.

### Stdlib
- `io.print/eprint`, `math.*`, `time.now()`,
  `fs.read/write/append/exists/delete`, `http.get/post` (HTTPS).

### Runtime
- Tesis medible: 0% CPU en reposo (handlers aparcados, sin polling — los
  canales usan Condvar), ~570k señales/seg en el intérprete.
- Supervisores con `one_for_one` y reinicios acotados.

### Herramientas
- Diagnósticos con snippet de código, subrayado `^^^` y pistas `help:`.
- 52 tests unitarios + 11 ejemplos golden ejecutados en `cargo test`.
- Benchmark reproducible en `bench/bench.ps1`.
- Extensión VS Code (resaltado, snippets, diagnósticos).
