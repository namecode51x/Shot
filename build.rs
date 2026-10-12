use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");

    let icon = Path::new("assets/icon.ico");
    if !icon.is_file() {
        println!(
            "cargo:warning=assets/icon.ico not found; the executable will keep the default icon"
        );
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let rc = out_dir.join("nameshot.rc");
    std::fs::write(&rc, "1 ICON \"assets/icon.ico\"\n").expect("write nameshot.rc");

    let compiled = match env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => {
            let res = out_dir.join("nameshot_icon.res");
            if run("rc.exe", &["/nologo", "/fo", utf8(&res), utf8(&rc)]) {
                Some(res)
            } else {
                None
            }
        }
        _ => {
            let obj = out_dir.join("nameshot_icon.o");
            for tool in ["x86_64-w64-mingw32-windres", "windres"] {
                if run(tool, &["-i", utf8(&rc), "-O", "coff", "-o", utf8(&obj)]) {
                    return link(&obj);
                }
            }
            None
        }
    };

    match compiled {
        Some(path) => link(&path),
        None => println!(
            "cargo:warning=no resource compiler (windres/rc.exe) on PATH; the executable will keep the default icon"
        ),
    }
}

fn link(path: &Path) {
    println!("cargo:rustc-link-arg-bins={}", path.display());
}

fn run(tool: &str, args: &[&str]) -> bool {
    Command::new(tool)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn utf8(path: &Path) -> &str {
    path.to_str().expect("path is valid UTF-8")
}
