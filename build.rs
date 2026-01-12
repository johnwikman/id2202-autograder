use static_files::resource_dir;
use std::env;
use std::path::Path;

fn main() -> std::io::Result<()> {
    let out_dir = env::var("OUT_DIR").unwrap();

    let mut rd = resource_dir("./web/static");
    rd.with_generated_filename(Path::new(&out_dir).join("generated_web_static.rs"));
    rd.build()?;

    Ok(())
}
