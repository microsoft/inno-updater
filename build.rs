use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
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
}
