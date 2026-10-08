fn main() {
    // The service manager runs this after the daemon stops, however it stopped.
    let result = if std::env::args().nth(1).as_deref() == Some("--restore-cooling") {
        prism_rust::daemon::restore_cooling()
    } else {
        prism_rust::daemon::run()
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
