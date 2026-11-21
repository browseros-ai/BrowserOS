//! `browseros_server`: the binary the browser launches. Everything is in `app::main`, which
//! returns an exit code rather than calling `exit`, so destructors run and the log is flushed.

fn main() -> std::process::ExitCode {
    browseros_server::app::main()
}
