#![windows_subsystem = "windows"]

fn main() {
    // Diagnostic aid: a panicking wndproc would otherwise die without a trace,
    // because a GUI-subsystem process has no console to write to.
    std::panic::set_hook(Box::new(|info| {
        let msg = format!(
            "nameshot panic: {}\nbacktrace:\n{}\n",
            info,
            std::backtrace::Backtrace::force_capture()
        );
        let _ = std::fs::write(
            std::env::temp_dir().join("nameshot_panic.txt"),
            msg.as_bytes(),
        );
        eprint!("{msg}");
    }));
    std::process::exit(nameshot::run());
}
