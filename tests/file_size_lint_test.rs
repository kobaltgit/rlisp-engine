// tests/file_size_lint_test.rs — Тест ограничения размера файлов (File Size Guard)
//
// Регламент AI_AGENTS.md: Ни один файл исходного кода не должен превышать 800 строк.
// Целевой ориентир — до 500 строк.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_LINES_LIMIT: usize = 800;

// Список монолитов, находящихся в процессе поэтапной декомпозиции (по мере рефакторинга
// файлы удаляются из этого списка, пока список не станет пустым):
const PENDING_REFACTORING: &[&str] = &[
    "src/gpu_3d.rs",
];

fn count_lines(path: &Path) -> usize {
    let content = fs::read_to_string(path).unwrap_or_default();
    content.lines().count()
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, out);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
}

#[test]
fn test_no_file_exceeds_size_limit() {
    let src_dir = Path::new("src");
    let mut files = Vec::new();
    collect_rs_files(src_dir, &mut files);

    let mut violations = Vec::new();

    for file in files {
        let norm_path = file.to_string_lossy().replace('\\', "/");
        if PENDING_REFACTORING.iter().any(|&p| norm_path == p) {
            continue;
        }

        let lines = count_lines(&file);
        if lines > MAX_LINES_LIMIT {
            violations.push(format!("{} ({} строк > лимит {})", norm_path, lines, MAX_LINES_LIMIT));
        }
    }

    assert!(
        violations.is_empty(),
        "\n\n[AI_AGENTS.md VIOLATION] Обнаружены файлы, превышающие лимит {} строк:\n{}\n\nПожалуйста, декомпозируйте файл на модули в соответствии с AI_AGENTS.md!\n",
        MAX_LINES_LIMIT,
        violations.join("\n")
    );
}
