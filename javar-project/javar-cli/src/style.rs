//! Terminal styling — indigo + orange accent.

use colored::Colorize;
use std::fmt::Display;

pub fn banner_line(msg: impl Display) {
    eprintln!("{}", format!("◆ {msg}").truecolor(99, 102, 241)); // indigo-500
}

pub fn ok(msg: impl Display) {
    eprintln!("{}", format!("✓ {msg}").truecolor(251, 146, 60)); // orange-400
}

pub fn warn_line(msg: impl Display) {
    eprintln!("{}", format!("! {msg}").truecolor(251, 146, 60).bold());
}

pub fn info_line(msg: impl Display) {
    eprintln!("{}", format!("· {msg}").truecolor(165, 180, 252)); // indigo-300
}

pub fn header(msg: impl Display) {
    eprintln!();
    eprintln!("{}", format!("  {msg}  ").bold().truecolor(99, 102, 241).on_truecolor(30, 27, 75));
    eprintln!();
}

pub fn accent(msg: impl Display) -> String {
    format!("{msg}").truecolor(251, 146, 60).to_string()
}
