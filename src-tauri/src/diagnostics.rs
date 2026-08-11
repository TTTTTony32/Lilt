use std::fmt::Display;

fn emit(level: &str, message: impl Display) {
    #[cfg(debug_assertions)]
    eprintln!("[lilt][backend][{level}] {message}");

    #[cfg(not(debug_assertions))]
    {
        let _ = (level, message);
    }
}

pub fn info(message: impl Display) {
    emit("INFO", message);
}

pub fn warn(message: impl Display) {
    emit("WARN", message);
}

pub fn error(message: impl Display) {
    emit("ERROR", message);
}
