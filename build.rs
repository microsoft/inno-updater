use std::process::Command;

fn main() {
    let mut rc = Command::new(".\\tools\\rc.exe")
                         .arg("/r")
                         .arg("/fo")
                         .arg(".\\resources\\resources.lib")
                         .arg(".\\resources\\resources.rc")
                         .spawn()
                         .expect("Failed to spawn resource compiler");

    let ecode = rc
                .wait()
                .expect("Failed to wait on resource compiler");

    assert!(ecode.success(), "Resource compiler failed");

    println!("cargo:rustc-link-lib=static=.\\resources\\resources");
}