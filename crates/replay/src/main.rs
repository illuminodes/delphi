mod fetcher;

#[allow(dead_code)]
fn _link_fetcher(f: &fetcher::Fetcher) -> std::path::PathBuf {
    f.headers_path()
}

fn main() {
    todo!("replay loop — see docs/architecture/replay.md")
}
