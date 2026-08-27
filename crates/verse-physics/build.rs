// SPDX-License-Identifier: AGPL-3.0-or-later

fn main() {
    // joltc-sys 0.3.1 builds Jolt/JoltC as static C++ libraries but does not
    // propagate their C++ runtime link requirement to final Rust binaries.
    // Keep that upstream packaging workaround local to this adapter.
    let target = std::env::var("TARGET").expect("Cargo always sets TARGET for build scripts");
    if target.contains("apple") {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else if target.contains("linux") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
