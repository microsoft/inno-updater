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
    
    // Get the current git commit hash
    let commit_hash = Command::new("git")
        .args(&["rev-parse", "HEAD"])
        .output()
        .map(|output| {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if hash.len() >= 10 {
                hash[..10].to_string()
            } else {
                hash
            }
        })
        .unwrap_or_else(|_| "unknown".to_string());
    //
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
        .replace("{{VERSION_STRING}}", &version)
        .replace("{{COMMIT}}", &commit_hash);
    
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
    
    // Make sure we rerun the build script when resources, cargo.toml, or git head changes
    println!("cargo:rerun-if-changed=resources/resources.rc.template");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=.git/HEAD");
}
