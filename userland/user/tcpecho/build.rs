//! 用户程序链接脚本接线。
//!
//! 只做一件事：把 `user.ld` 交给最终链接（`cargo:rustc-link-arg-bins`），
//! 让用户程序固定链接在 `USER_VIRT_BASE`（0x0000_0080_0000_0000）上 ——
//! 内核的 `load_program` 把映像原样拷到该地址后直接取指，不做重定位。
fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    println!("cargo:rustc-link-arg-bins=-T{dir}/user.ld");
    println!("cargo:rerun-if-changed=user.ld");
}
