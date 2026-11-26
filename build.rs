fn main() {
    // C++ spreadsheet engine.
    cc::Build::new()
        .include("include")
        .cpp(true)
        .file("native/rx_core.cpp")
        .flag_if_supported("-std=c++17")
        .warnings(false)
        .compile("rx_core");

    // Plain C CSV helper.
    cc::Build::new()
        .include("include")
        .file("native/rx_csv.c")
        .warnings(false)
        .compile("rx_csv");

    println!("cargo:rerun-if-changed=include/rx_core.h");
    println!("cargo:rerun-if-changed=native/rx_core.cpp");
    println!("cargo:rerun-if-changed=native/rx_csv.c");
}
