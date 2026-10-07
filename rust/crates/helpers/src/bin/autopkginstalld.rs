fn main() {
    if let Err(error) = autopkg_helpers::run(autopkg_helpers::Service::Installation) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
