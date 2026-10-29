//! Compiles the reviewed OCR source into allocation-free static descriptors.

mod catalog_build;

fn main() {
    println!("cargo:rerun-if-changed=resources/model-catalog.json");
    println!("cargo:rerun-if-changed=catalog_build.rs");
    let source =
        std::fs::read_to_string("resources/model-catalog.json").expect("read OCR model catalog");
    let catalog = catalog_build::validate(&source).expect("invalid OCR model catalog");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    std::fs::write(
        out.join("model_catalog.rs"),
        catalog.rust_source().expect("emit OCR catalog"),
    )
    .expect("write compiled OCR catalog");
}
