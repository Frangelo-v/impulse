mod ast;
mod diagnostics;
mod execution;
mod lexer;
mod parser;
mod runtime;
mod semantic;
#[cfg(test)]
mod tests;

use clap::Parser as ClapParser;
use std::path::PathBuf;

#[derive(ClapParser)]
#[command(
    name = "impulsec",
    about = "The Impulse language compiler",
    version = env!("CARGO_PKG_VERSION")
)]
struct Cli {
    /// Source file to compile or run
    file: PathBuf,

    /// Print the token stream and exit
    #[arg(long)]
    emit_tokens: bool,

    /// Print the parsed AST and exit
    #[arg(long)]
    emit_ast: bool,

    /// Print the signal dependency graph and exit
    #[arg(long)]
    emit_graph: bool,

    /// Run frontend and semantic analysis without executing
    #[arg(long)]
    check: bool,

    /// Print runtime activation statistics after execution
    #[arg(long)]
    runtime_stats: bool,
}

fn main() {
    let cli = Cli::parse();
    let source = read_source(&cli.file);
    let tokens = lexer::tokenize(&source);

    if cli.emit_tokens {
        for token in &tokens {
            println!("{:?}", token);
        }
        return;
    }

    let program = parser::parse(tokens, &source).unwrap_or_else(|err| {
        let (message, span) = diagnostics::split_parse_span(&err);
        diagnostics::print(
            &cli.file,
            &source,
            span.unwrap_or(0..0),
            "error",
            &format!("parse error: {}", message),
        );
        std::process::exit(1);
    });

    if cli.emit_ast {
        println!("{:#?}", program);
        return;
    }

    let analysis = semantic::analyze(&program);
    for warning in &analysis.warnings {
        diagnostics::print(
            &cli.file,
            &source,
            warning.span.clone(),
            "warning",
            &warning.message,
        );
    }

    if !analysis.errors.is_empty() {
        for error in &analysis.errors {
            diagnostics::print(
                &cli.file,
                &source,
                error.span.clone(),
                "error",
                &error.message,
            );
        }
        eprintln!("impulsec: {} error(s) found", analysis.errors.len());
        std::process::exit(1);
    }

    if cli.emit_graph {
        print_signal_graph(&analysis.signal_graph);
        return;
    }

    if cli.check {
        eprintln!("impulsec: check passed - 0 errors");
        return;
    }

    let options = execution::RunOptions {
        emit_stats: cli.runtime_stats,
    };
    if let Err(err) = execution::run_with_options(&program, options) {
        eprintln!("impulsec: runtime error: {}", err);
        std::process::exit(1);
    }
}

fn read_source(path: &PathBuf) -> String {
    let source = std::fs::read_to_string(path).unwrap_or_else(|err| {
        eprintln!("impulsec: cannot read '{}': {}", path.display(), err);
        std::process::exit(1);
    });
    // Editors on Windows often prepend a UTF-8 BOM; it would corrupt
    // spans and column numbers in diagnostics.
    source.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(source)
}

fn print_signal_graph(graph: &semantic::SignalGraph) {
    println!("=== Signal Dependency Graph ===");
    for (signal, listeners) in &graph.listeners {
        let recursive = if graph.recursive.contains(signal) {
            " [recursive]"
        } else {
            ""
        };
        println!("signal {}{}", signal, recursive);
        for listener in listeners {
            println!("  - surge: {}", listener);
        }
    }

    println!("\n=== Declared Signals with no listeners ===");
    for signal in graph.modes.keys() {
        if !graph.listeners.contains_key(signal) {
            println!("  ! {}", signal);
        }
    }
}
