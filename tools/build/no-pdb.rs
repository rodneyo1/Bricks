// Debug information is disabled by the workspace profiles. MSVC otherwise still
// emits large PDB files for these targets. Dependency symbols are unaffected.
pub fn configure() {
    if std::env::var("TARGET").is_ok_and(|target| target.ends_with("windows-msvc")) {
        println!("cargo:rustc-link-arg=/DEBUG:NONE");
    }
}
