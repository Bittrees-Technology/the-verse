// SPDX-License-Identifier: AGPL-3.0-or-later

fn main() {
    let target = std::env::var("TARGET").expect("Cargo always sets TARGET for build scripts");
    if target.contains("linux") {
        // joltc-sys 0.3.1 publishes Jolt and JoltC as separate static archives.
        // The dependency metadata reaches a release binary as `Jolt, joltc`,
        // while JoltC depends on Jolt and both depend on the C++ runtime. GNU
        // ld's single left-to-right archive pass therefore leaves symbols
        // unresolved. Repeat the two pinned archives as one bounded group at
        // the final binary boundary, followed by GCC's static compiler helpers
        // (needed by outlined AArch64 atomics) and the C++ runtime. This is not
        // needed by Apple's archive linker.
        println!("cargo:rustc-link-arg-bin=verse-simulation-worker=-Wl,--start-group");
        println!("cargo:rustc-link-arg-bin=verse-simulation-worker=-ljoltc");
        println!("cargo:rustc-link-arg-bin=verse-simulation-worker=-lJolt");
        println!("cargo:rustc-link-arg-bin=verse-simulation-worker=-Wl,--end-group");
        println!("cargo:rustc-link-arg-bin=verse-simulation-worker=-lgcc");
        println!("cargo:rustc-link-arg-bin=verse-simulation-worker=-lstdc++");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
