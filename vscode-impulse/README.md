# Impulse Language for VS Code

Local VS Code support for Impulse:

- `.imp` language detection
- TextMate syntax highlighting
- `Impulse Synapse Bloom` color theme
- snippets for `signal`, `when`, `node`, `surge`, `actor`, and `supervisor`

## Visual Language

The theme avoids generic keyword coloring. It treats Impulse code as a living propagation graph:

- Signals glow electric cyan.
- Reactive handlers bloom coral.
- Pulses and lifecycle operations burn amber.
- Actors, clusters, and domains sit in violet.
- Shared state is cool mint.
- Errors discharge in red.

## Development Install

From VS Code:

1. Open this `vscode-impulse` folder.
2. Press `F5`.
3. In the Extension Development Host, open an `.imp` file.
4. Select `Impulse Synapse Bloom` from `Preferences: Color Theme`.

For a permanent install, package it later with `vsce package`.
