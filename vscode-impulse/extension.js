const vscode = require("vscode");
const cp = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

let output;
let diagnostics;
const timers = new Map();
const runningChecks = new Map();
let resolvedCompilerCache = null;

function activate(context) {
  output = vscode.window.createOutputChannel("Impulse");
  diagnostics = vscode.languages.createDiagnosticCollection("impulse");
  context.subscriptions.push(output);
  context.subscriptions.push(diagnostics);

  context.subscriptions.push(
    vscode.commands.registerCommand("impulse.checkFile", () => runImpulse(["--check"], "check")),
    vscode.commands.registerCommand("impulse.emitGraph", () => runImpulse(["--emit-graph"], "signal graph")),
    vscode.commands.registerCommand("impulse.emitAst", () => runImpulse(["--emit-ast"], "AST")),
    vscode.commands.registerCommand("impulse.emitTokens", () => runImpulse(["--emit-tokens"], "tokens")),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("impulse.compilerPath")) {
        resolvedCompilerCache = null;
      }
    }),
    vscode.workspace.onDidChangeTextDocument((event) => {
      const cfg = vscode.workspace.getConfiguration("impulse");
      if (cfg.get("checkOnType")) {
        scheduleDiagnostics(event.document, cfg.get("checkDelayMs") || 1200);
      }
    }),
    vscode.workspace.onDidSaveTextDocument((doc) => {
      const cfg = vscode.workspace.getConfiguration("impulse");
      if (cfg.get("checkOnSave") && doc.languageId === "impulse") {
        updateDiagnostics(doc, false);
      }
    }),
    vscode.workspace.onDidCloseTextDocument((doc) => {
      diagnostics.delete(doc.uri);
      cancelRunningCheck(doc.uri.toString());
    })
  );
}

function deactivate() {
  for (const [key, child] of runningChecks) {
    if (!child.killed) { child.kill(); }
  }
  runningChecks.clear();
  for (const [key, timer] of timers) {
    clearTimeout(timer);
  }
  timers.clear();
  resolvedCompilerCache = null;
}

async function runImpulse(args, label, doc) {
  const document = doc || vscode.window.activeTextEditor?.document;
  if (!document || document.languageId !== "impulse") {
    vscode.window.showWarningMessage("Open an Impulse .imp file first.");
    return;
  }

  if (document.isDirty) {
    await document.save();
  }

  const file = document.uri.fsPath;
  const command = resolveCompiler(file);
  output.clear();
  output.show(true);
  output.appendLine(`Impulse ${label}`);
  output.appendLine(`> ${command.display} ${quote(file)} ${args.join(" ")}`);
  output.appendLine("");

  const child = cp.spawn(command.exe, [...command.prefixArgs, file, ...args], {
    cwd: command.cwd,
    shell: false
  });

  child.stdout.on("data", (data) => output.append(data.toString()));
  child.stderr.on("data", (data) => output.append(data.toString()));
  child.on("error", (err) => {
    output.appendLine(`Failed to start impulsec: ${err.message}`);
    vscode.window.showErrorMessage("Impulse compiler could not be started. Set impulse.compilerPath.");
  });
  child.on("close", (code) => {
    if (code === 0) {
      vscode.window.setStatusBarMessage(`Impulse ${label} passed`, 3000);
    } else {
      vscode.window.showErrorMessage(`Impulse ${label} failed. See Impulse output.`);
    }
  });
}

function scheduleDiagnostics(doc, delay) {
  if (!doc || doc.languageId !== "impulse" || doc.uri.scheme !== "file") {
    return;
  }

  const key = doc.uri.toString();
  const existing = timers.get(key);
  if (existing) {
    clearTimeout(existing);
  }

  timers.set(
    key,
    setTimeout(() => {
      timers.delete(key);
      updateDiagnostics(doc, false);
    }, delay)
  );
}

async function updateDiagnostics(doc, showOutput) {
  if (!doc || doc.languageId !== "impulse" || doc.uri.scheme !== "file") {
    return;
  }

  const cfg = vscode.workspace.getConfiguration("impulse");
  const maxKb = cfg.get("maxCheckFileKb") || 512;
  if (Buffer.byteLength(doc.getText(), "utf8") > maxKb * 1024) {
    diagnostics.delete(doc.uri);
    return;
  }

  const file = doc.uri.fsPath;
  const checkFile = await materializeDocumentForCheck(doc);
  const command = resolveCompiler(file);
  const key = doc.uri.toString();
  cancelRunningCheck(key);
  const result = await runProcess(command, [checkFile, "--check"], key);
  if (result.cancelled) {
    if (checkFile !== file) {
      fs.rm(checkFile, { force: true }, () => {});
    }
    return;
  }
  const text = `${result.stdout}\n${result.stderr}`;
  const parsed = parseDiagnostics(doc, text, result.code);

  diagnostics.set(doc.uri, parsed);

  if (showOutput) {
    output.clear();
    output.show(true);
    output.appendLine(`Impulse diagnostics`);
    output.appendLine(`> ${command.display} ${quote(checkFile)} --check`);
    output.appendLine("");
    output.append(text.trim() || "No output.");
  }

  if (checkFile !== file) {
    fs.rm(checkFile, { force: true }, () => {});
  }
}

async function materializeDocumentForCheck(doc) {
  const file = doc.uri.fsPath;
  if (!doc.isDirty) {
    return file;
  }

  const dir = path.join(os.tmpdir(), "impulse-vscode");
  await fs.promises.mkdir(dir, { recursive: true });
  const name = `${path.basename(file, ".imp")}-${Date.now()}-${Math.random().toString(16).slice(2)}.imp`;
  const tempFile = path.join(dir, name);
  await fs.promises.writeFile(tempFile, doc.getText(), "utf8");
  return tempFile;
}

function runProcess(command, args, key) {
  return new Promise((resolve) => {
    const child = cp.spawn(command.exe, [...command.prefixArgs, ...args], {
      cwd: command.cwd,
      shell: false
    });
    if (key) {
      runningChecks.set(key, child);
    }

    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (data) => {
      stdout += data.toString();
    });
    child.stderr.on("data", (data) => {
      stderr += data.toString();
    });
    child.on("error", (err) => {
      if (key) {
        runningChecks.delete(key);
      }
      resolve({
        code: 127,
        stdout: "",
        stderr: `error: failed to start impulsec: ${err.message}`
      });
    });
    child.on("close", (code) => {
      if (key) {
        runningChecks.delete(key);
      }
      resolve({ code, stdout, stderr, cancelled: child.killed });
    });
  });
}

function cancelRunningCheck(key) {
  const child = runningChecks.get(key);
  if (child && !child.killed) {
    child.kill();
  }
  runningChecks.delete(key);
}

function parseDiagnostics(doc, text, exitCode) {
  const items = [];
  const lines = text.split(/\r?\n/).filter(Boolean);

  for (const line of lines) {
    const located = line.match(/^(.+?):(\d+):(\d+):(\d+):(\d+):\s*(error|warning):\s*(.+)$/i);
    if (located) {
      const severity = located[6].toLowerCase() === "error"
        ? vscode.DiagnosticSeverity.Error
        : vscode.DiagnosticSeverity.Warning;
      items.push(makeLocatedDiagnostic(
        doc,
        located[7],
        severity,
        Number(located[2]),
        Number(located[3]),
        Number(located[4]),
        Number(located[5])
      ));
      continue;
    }

    const error = line.match(/^error:\s*(.+)$/i) || line.match(/^impulsec:\s*parse error:\s*(.+)$/i);
    const warning = line.match(/^warning:\s*(.+)$/i);

    if (error) {
      items.push(makeSmartDiagnostic(doc, error[1], vscode.DiagnosticSeverity.Error));
    } else if (warning) {
      items.push(makeSmartDiagnostic(doc, warning[1], vscode.DiagnosticSeverity.Warning));
    }
  }

  if (exitCode !== 0 && items.length === 0) {
    items.push(makeSmartDiagnostic(doc, "Impulse check failed. See Impulse output.", vscode.DiagnosticSeverity.Error));
  }

  return items;
}

function makeLocatedDiagnostic(doc, message, severity, line, col, endLine, endCol) {
  const startLine = clamp(line - 1, 0, doc.lineCount - 1);
  const finishLine = clamp(endLine - 1, startLine, doc.lineCount - 1);
  const startChar = clamp(col - 1, 0, doc.lineAt(startLine).text.length);
  let endChar = clamp(endCol - 1, 0, doc.lineAt(finishLine).text.length);

  if (startLine === finishLine && endChar <= startChar) {
    endChar = Math.min(doc.lineAt(startLine).text.length, startChar + 1);
  }

  const diagnostic = new vscode.Diagnostic(
    new vscode.Range(startLine, startChar, finishLine, endChar),
    message,
    severity
  );
  diagnostic.source = "impulsec";
  return diagnostic;
}

function makeSmartDiagnostic(doc, message, severity) {
  const tokenPos = message.match(/\bat pos (\d+)\b/i);
  if (tokenPos) {
    const token = tokenAt(doc.getText(), Number(tokenPos[1]));
    if (token) {
      return makeLocatedDiagnostic(
        doc,
        cleanMessage(message),
        severity,
        token.line + 1,
        token.col + 1,
        token.endLine + 1,
        token.endCol + 1
      );
    }
  }

  const quoted = message.match(/['`]([A-Za-z_][A-Za-z0-9_:]*)['`]/);
  if (quoted) {
    const found = findIdentifierLike(doc, quoted[1]);
    if (found) {
      return makeLocatedDiagnostic(
        doc,
        message,
        severity,
        found.line + 1,
        found.col + 1,
        found.endLine + 1,
        found.endCol + 1
      );
    }
  }

  return makeDiagnostic(doc, message, severity);
}

function makeDiagnostic(doc, message, severity) {
  const line = firstUsefulLine(doc);
  const text = doc.lineAt(line).text;
  const firstNonSpace = Math.max(0, text.search(/\S/));
  const start = firstNonSpace === -1 ? 0 : firstNonSpace;
  const end = Math.max(start + 1, text.length);
  const diagnostic = new vscode.Diagnostic(
    new vscode.Range(line, start, line, end),
    message,
    severity
  );
  diagnostic.source = "impulsec";
  return diagnostic;
}

function firstUsefulLine(doc) {
  for (let i = 0; i < doc.lineCount; i += 1) {
    const text = doc.lineAt(i).text.trim();
    if (text && !text.startsWith("//")) {
      return i;
    }
  }
  return 0;
}

function tokenAt(text, index) {
  const tokens = scanTokens(text);
  if (index < tokens.length) {
    return tokens[index];
  }
  return tokens[tokens.length - 1];
}

function findIdentifierLike(doc, value) {
  const needles = value.includes("::") ? value.split("::") : [value];
  const tokens = scanTokens(doc.getText());
  for (const token of tokens) {
    if (needles.includes(token.text)) {
      return token;
    }
  }
  return null;
}

function scanTokens(text) {
  const tokens = [];
  let i = 0;
  let line = 0;
  let col = 0;

  const push = (start, end, startLine, startCol, endLine, endCol) => {
    tokens.push({
      text: text.slice(start, end),
      line: startLine,
      col: startCol,
      endLine,
      endCol
    });
  };

  const advance = () => {
    const ch = text[i++];
    if (ch === "\n") {
      line += 1;
      col = 0;
    } else {
      col += 1;
    }
    return ch;
  };

  while (i < text.length) {
    const ch = text[i];
    if (/\s/.test(ch)) {
      advance();
      continue;
    }
    if (ch === "/" && text[i + 1] === "/") {
      while (i < text.length && text[i] !== "\n") {
        advance();
      }
      continue;
    }
    if (ch === "/" && text[i + 1] === "*") {
      advance();
      advance();
      while (i < text.length && !(text[i] === "*" && text[i + 1] === "/")) {
        advance();
      }
      if (i < text.length) {
        advance();
        advance();
      }
      continue;
    }

    const start = i;
    const startLine = line;
    const startCol = col;

    if (ch === "\"") {
      advance();
      while (i < text.length) {
        const current = advance();
        if (current === "\\") {
          advance();
        } else if (current === "\"") {
          break;
        }
      }
      push(start, i, startLine, startCol, line, col);
      continue;
    }

    if (/[A-Za-z_]/.test(ch)) {
      advance();
      while (i < text.length && /[A-Za-z0-9_]/.test(text[i])) {
        advance();
      }
      push(start, i, startLine, startCol, line, col);
      continue;
    }

    if (/[0-9]/.test(ch)) {
      advance();
      while (i < text.length && /[A-Za-z0-9_.]/.test(text[i])) {
        advance();
      }
      push(start, i, startLine, startCol, line, col);
      continue;
    }

    const two = text.slice(i, i + 2);
    const three = text.slice(i, i + 3);
    if (["..=", "**="].includes(three)) {
      advance();
      advance();
      advance();
    } else if (["**", "<<", ">>", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", "|>", "->", "<-", "..", "??", "::"].includes(two)) {
      advance();
      advance();
    } else {
      advance();
    }
    push(start, i, startLine, startCol, line, col);
  }

  return tokens;
}

function cleanMessage(message) {
  return message.replace(/\s+at pos \d+\b/i, "");
}

function resolveCompiler(file) {
  // Return cached result to avoid repeated disk I/O on every diagnostic run
  if (resolvedCompilerCache) {
    return resolvedCompilerCache;
  }

  const cfg = vscode.workspace.getConfiguration("impulse");
  const configured = process.env.IMPULSE_COMPILER || cfg.get("compilerPath") || "impulsec";
  const folder = vscode.workspace.getWorkspaceFolder(vscode.Uri.file(file));
  const workspace = folder?.uri.fsPath || path.dirname(file);

  if (configured && configured !== "impulsec") {
    resolvedCompilerCache = { exe: configured, prefixArgs: [], cwd: workspace, display: configured };
    return resolvedCompilerCache;
  }

  // Check pre-built binaries — GNU target first (Windows default), then MSVC, then release
  const exe = exeName("impulsec");
  const appData = process.env.LOCALAPPDATA || path.join(os.homedir(), "AppData", "Local");
  const candidates = [
    // AppData target (outside OneDrive, no WDAC restrictions)
    path.join(appData, "impulse-target", "x86_64-pc-windows-gnu", "release", exe),
    path.join(appData, "impulse-target", "x86_64-pc-windows-gnu", "debug", exe),
    path.join(appData, "impulse-target", "release", exe),
    path.join(appData, "impulse-target", "debug", exe),
    // Workspace-local targets
    path.join(workspace, "compiler", "target", "x86_64-pc-windows-gnu", "release", exe),
    path.join(workspace, "compiler", "target", "x86_64-pc-windows-gnu", "debug", exe),
    path.join(workspace, "compiler", "target", "x86_64-pc-windows-msvc", "release", exe),
    path.join(workspace, "compiler", "target", "x86_64-pc-windows-msvc", "debug", exe),
    path.join(workspace, "compiler", "target", "release", exe),
    path.join(workspace, "compiler", "target", "debug", exe),
    path.join(workspace, "target", "release", exe),
    path.join(workspace, "target", "debug", exe),
  ];

  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) {
      resolvedCompilerCache = { exe: candidate, prefixArgs: [], cwd: workspace, display: candidate };
      return resolvedCompilerCache;
    }
  }

  // Fallback: build once with cargo (heavy — only if no binary exists yet)
  const manifest = path.join(workspace, "compiler", "Cargo.toml");
  if (fs.existsSync(manifest)) {
    // Build the binary first so future runs are fast
    const result = {
      exe: "cargo",
      prefixArgs: [
        "+stable-x86_64-pc-windows-gnu",
        "run",
        "-q",
        "--target", "x86_64-pc-windows-gnu",
        "--manifest-path", manifest,
        "--",
      ],
      cwd: workspace,
      display: "cargo run (building...)"
    };
    // Don't cache cargo run — try again next time in case binary was built
    return result;
  }

  return { exe: configured, prefixArgs: [], cwd: workspace, display: configured };
}

function exeName(name) {
  return process.platform === "win32" ? `${name}.exe` : name;
}

function quote(value) {
  return value.includes(" ") ? `"${value}"` : value;
}

function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}

module.exports = {
  activate,
  deactivate
};
