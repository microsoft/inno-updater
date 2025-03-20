use std::env;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Command;

fn main() {
    // Get the package version from Cargo.toml through environment variables
    let version = env::var("CARGO_PKG_VERSION").expect("Failed to get package version");
    
    // Parse the version components
    let version_parts: Vec<&str> = version.split('.').collect();
    if version_parts.len() < 3 {
        panic!("Version format must be 'major.minor.patch'");
    }
    
    let major = version_parts[0];
    let minor = version_parts[1];
    let patch = version_parts[2];
    
    // Read the resource template
    let mut template_content = String::new();
    fs::File::open("resources/resources.rc.template")
        .expect("Failed to open resources.rc.template")
        .read_to_string(&mut template_content)
        .expect("Failed to read resources.rc.template");
    
    // Replace version placeholders
    let updated_content = template_content
        .replace("{{VERSION_MAJOR}}", major)
        .replace("{{VERSION_MINOR}}", minor)
        .replace("{{VERSION_PATCH}}", patch)
        .replace("{{VERSION_STRING}}", &version);
    
    // Write the updated content to resources.rc
    fs::write("resources/resources.rc", updated_content)
        .expect("Failed to write resources.rc");
    
    // Continue with resource compilation
    let out_dir = env::var("OUT_DIR").expect("Missing out directory?");
    let resources = Path::new(&out_dir).join("resources.lib");

    let ecode = Command::new(".\\tools\\rc.exe")
        .arg("/r")
        .arg("/fo")
        .arg(resources.as_os_str())
        .arg(".\\resources\\resources.rc")
        .spawn()
        .expect("Failed to spawn resource compiler")
        .wait()
        .expect("Failed to wait on resource compiler");

    assert!(ecode.success(), "Resource compiler failed");

    println!(
        "cargo:rustc-link-search=native={}",
        resources.parent().unwrap().to_str().unwrap()
    );
    println!(
        "cargo:rustc-link-lib={}",
        resources.file_stem().unwrap().to_str().unwrap()
    );
    
    // Make sure we rerun the build script when resources or cargo.toml changes
    println!("cargo:rerun-if-changed=resources/resources.rc.template");
    println!("cargo:rerun-if-changed=Cargo.toml");
}
