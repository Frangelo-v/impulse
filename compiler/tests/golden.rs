//! Golden tests: ejecutan cada examples/*.imp que tenga un .expected al lado
//! y comparan la salida estándar exacta. Si cambias el comportamiento del
//! lenguaje a propósito, regenera el .expected y revísalo a mano.

use std::path::{Path, PathBuf};
use std::process::Command;

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("examples")
}

fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n")
}

#[test]
fn golden_examples() {
    let dir = examples_dir();
    let mut checked = 0;
    let mut failures = Vec::new();

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("no se pudo leer examples/")
        .map(|e| e.expect("entrada de directorio").path())
        .collect();
    entries.sort();

    for path in entries {
        if path.extension().map(|e| e == "imp") != Some(true) {
            continue;
        }
        let expected_path = path.with_extension("expected");
        if !expected_path.exists() {
            continue; // ejemplos sin .expected (p. ej. los que no terminan) se omiten
        }
        let expected = normalize(&std::fs::read_to_string(&expected_path).unwrap());
        let output = Command::new(env!("CARGO_BIN_EXE_impulsec"))
            .arg(&path)
            .output()
            .expect("no se pudo ejecutar impulsec");
        let stdout = normalize(&String::from_utf8_lossy(&output.stdout));
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        if !output.status.success() {
            failures.push(format!(
                "{}: terminó con código {:?}\nstderr:\n{}",
                name,
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            ));
        } else if stdout != expected {
            failures.push(format!(
                "{}: la salida no coincide\n--- esperado ---\n{}--- obtenido ---\n{}",
                name, expected, stdout
            ));
        }
        checked += 1;
    }

    assert!(checked > 0, "no se encontró ningún golden test en {}", dir.display());
    assert!(
        failures.is_empty(),
        "{} golden test(s) fallaron:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    println!("golden: {} ejemplos verificados", checked);
}
