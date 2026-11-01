fn main() {
    if let Err(error) = nya_scraping_core::cli::run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
