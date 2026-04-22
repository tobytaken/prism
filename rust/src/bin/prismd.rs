fn main() {
    if let Err(error) = prism_rust::daemon::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
