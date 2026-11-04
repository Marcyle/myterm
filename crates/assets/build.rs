fn main() {
    // rust-embed 的 proc macro 不会声明 rerun-if-changed:往 assets 目录新增/修改
    // 文件不会触发本 crate 重编,导致 release 构建嵌入过期的资源快照。
    // 这里显式监听资源目录,保证图标/字体的增删改都能及时嵌入。
    println!("cargo:rerun-if-changed=assets");
}
