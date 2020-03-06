extern crate bindgen;

use std::{env, path::PathBuf};

fn main() {
    // Compile oppai
    cc::Build::new()
        .define("OPPAI_IMPLEMENTATION", None)
        .files(&["oppai-ng/oppai.c"])
        .compile("oppai");
    // Link to compiled oppai
    println!("cargo:rustc-link-lib=oppai");
    // Generate binding
    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .generate()
        .expect("Unable to generate bindings");
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
