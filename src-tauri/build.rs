fn main() {
    // tauri_build 的重跑条件只覆盖 tauri.conf.json 与 capabilities，
    // 前端产物（../build）变化不会触发 codegen，导致旧资源被打进二进制。
    // 这里显式声明：build/ 下任何文件变动都要重跑 build script。
    println!("cargo:rerun-if-changed=../build");

    tauri_build::build()
}
