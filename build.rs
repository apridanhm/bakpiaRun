#![allow(deprecated)]
use std::env;
use std::path::PathBuf;

fn main() {
    let php_prefix = "/usr/local/php-embed";
    let php_source = "/home/bakpiarun-dev/php-8.3.8";

    println!("cargo:rustc-link-search=native={}/lib", php_prefix);
    println!("cargo:rustc-link-lib=dylib=php");

    println!("cargo:rerun-if-changed=wrapper.h");

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        // Include paths untuk header yang udah ter-install
        .clang_arg(format!("-I{}/include/php", php_prefix))
        .clang_arg(format!("-I{}/include/php/Zend", php_prefix))
        .clang_arg(format!("-I{}/include/php/TSRM", php_prefix))
        .clang_arg(format!("-I{}/include/php/main", php_prefix))
        
        // Include paths untuk header di source code asli (Kunci buat nemuin php_embed.h!)
        .clang_arg(format!("-I{}/sapi/embed", php_source))
        .clang_arg(format!("-I{}/main", php_source))
        .clang_arg(format!("-I{}/Zend", php_source))
        .clang_arg(format!("-I{}/TSRM", php_source))
        
        // Blocklist konstanta matematika C yang duplicate
        .blocklist_item("FP_.*")
        
        .rust_target(bindgen::RustTarget::Stable_1_82)
        //.rust_target(bindgen::RustTarget::stable(82, 0).unwrap())
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}