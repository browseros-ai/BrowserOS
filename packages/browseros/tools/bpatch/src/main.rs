use clap::Parser;

fn main() {
    let cli = bpatch::cli::Cli::parse_from(bpatch::cli::normalize_args(std::env::args_os()));
    std::process::exit(bpatch::cli::run(cli));
}
