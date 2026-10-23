fn main() {
    // rust-embed resolves its folder at compile time. A fresh clone has no web
    // build yet, and `vite build` empties the directory, so no tracked
    // placeholder can hold it open.
    let dist = concat!(env!("CARGO_MANIFEST_DIR"), "/../../dist");
    std::fs::create_dir_all(dist).expect("create web asset directory");
    println!("cargo:rerun-if-changed={dist}");
}
