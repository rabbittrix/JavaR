//! Detect non-Maven source layouts and offer a one-shot fix.
//! Author: Roberto de Souza <rabbittrix@hotmail.com>

use crate::style;
use anyhow::{Context, Result};
use std::io::{self, Write};
use std::path::Path;

/// If `src/com` exists but `src/main/java` does not, offer to move sources
/// into the Maven-standard layout.
pub fn maybe_fix_src_com_layout(root: &Path) -> Result<()> {
    let src_com = root.join("src").join("com");
    let main_java = root.join("src").join("main").join("java");
    if !src_com.is_dir() || main_java.is_dir() {
        return Ok(());
    }

    println!();
    style::warn_line(
        "Structure alert: Your source files are in 'src/com'. \
         Maven expects them in 'src/main/java/com'. \
         Would you like JavaR to fix this for you? (y/n)",
    );
    print!("> ");
    let _ = io::stdout().flush();
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !answer.trim().eq_ignore_ascii_case("y") {
        style::info_line("Leaving layout unchanged.");
        return Ok(());
    }

    let dest = main_java.join("com");
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    move_tree(&src_com, &dest)?;
    // Remove empty src/com if possible
    let _ = std::fs::remove_dir(&src_com);
    style::ok(format!(
        "Moved {} → {}",
        src_com.display(),
        dest.display()
    ));
    Ok(())
}

fn move_tree(from: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        // Merge: move children into existing dest
        for entry in std::fs::read_dir(from).with_context(|| format!("read {}", from.display()))? {
            let entry = entry?;
            let src = entry.path();
            let name = entry.file_name();
            let dest = to.join(&name);
            if src.is_dir() {
                std::fs::create_dir_all(&dest)?;
                move_tree(&src, &dest)?;
                let _ = std::fs::remove_dir(&src);
            } else {
                if dest.exists() {
                    std::fs::remove_file(&dest)?;
                }
                std::fs::rename(&src, &dest)
                    .or_else(|_| copy_then_remove(&src, &dest))?;
            }
        }
    } else if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
        if std::fs::rename(from, to).is_err() {
            copy_dir_recursive(from, to)?;
            remove_dir_all_best(from);
        }
    }
    Ok(())
}

fn copy_then_remove(src: &Path, dest: &Path) -> Result<()> {
    std::fs::copy(src, dest)?;
    std::fs::remove_file(src)?;
    Ok(())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn remove_dir_all_best(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}
