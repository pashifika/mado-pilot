//! Validates a source catalog with the build validator and prints execution digests.

#[path = "../catalog_build.rs"]
mod catalog_build;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const USAGE: &str = "usage: ocr-catalog-validate [catalog.json [--emit-rust]] | --resources PROFILE_ID [catalog.json]";
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    let (resources, path, emit_rust) =
        if first.as_deref() == Some(std::ffi::OsStr::new("--resources")) {
            let id = args.next().ok_or(USAGE)?.into_string().map_err(|_| USAGE)?;
            (Some(id), args.next(), false)
        } else {
            let emit = match args.next() {
                None => false,
                Some(flag) if flag == "--emit-rust" => true,
                Some(_) => return Err(USAGE.into()),
            };
            (None, first, emit)
        };
    if args.next().is_some() {
        return Err(USAGE.into());
    }
    let path = path.map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/model-catalog.json")
    });
    // Bound the read, not only the already allocated parser input.
    let file = std::fs::File::open(path)?;
    let mut source = String::new();
    use std::io::Read as _;
    file.take(1024 * 1024 + 1).read_to_string(&mut source)?;
    let catalog = catalog_build::validate(&source)?;
    if let Some(id) = resources {
        for resource in catalog.resources(&id)? {
            println!(
                "{}\t{}\t{}\t{}",
                resource.relative_path, resource.byte_len, resource.sha256, resource.source_url
            );
        }
    } else if emit_rust {
        print!("{}", catalog.rust_source()?);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&catalog.execution_digests()?)?
        );
    }
    Ok(())
}
