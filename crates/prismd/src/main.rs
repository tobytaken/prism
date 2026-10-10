fn main() {
    // The service manager runs this after the daemon stops, however it stopped.
    let result = if std::env::args().nth(1).as_deref() == Some("--restore-cooling") {
        prismd::restore_cooling()
    } else {
        prismd::run()
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
