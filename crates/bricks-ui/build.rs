fn main() {
    include_symbols::configure();
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent-light".into()),
    )
    .expect("compile Bricks desktop interface");
}
#[path = "../../tools/build/no-pdb.rs"]
mod include_symbols;
