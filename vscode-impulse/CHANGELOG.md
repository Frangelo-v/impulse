# Changelog

## 0.1.4

- Make diagnostics nearly idle by default: automatic checks run on save, not on every keystroke.
- Add `impulse.checkDelayMs` and `impulse.maxCheckFileKb`.
- Stop opening the output panel automatically for background diagnostics.

## 0.1.3

- Add `when` as the preferred reactive handler keyword.
- Keep `on` highlighting as a compatibility alias.
- Update snippets to generate beginner-friendly `when` handlers.

## 0.1.2

- Improve live diagnostics so parser token positions are underlined more accurately.
- Prepare support for compiler diagnostics with exact line and column ranges.

## 0.1.1

- Add editor diagnostics powered by `impulsec --check`.
- Run diagnostics on save and shortly after typing by default.
- Add `impulse.checkOnType` configuration.

## 0.1.0

- Add Impulse syntax highlighting for `.imp` files.
- Add the Impulse Synapse Bloom theme.
- Add snippets for core language constructs.
- Add compiler commands for check, graph, AST, and tokens.
