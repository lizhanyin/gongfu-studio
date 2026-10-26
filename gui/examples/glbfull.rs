//! 整体 GLB 导出示例:AvatarGirl 基础 5 件 + girl.FSKEL 骨架动画 + FM01 帽子。
//! 用法: cargo run --example glbfull -- [输出.glb] [动作名(默认 girl.FSKEL 全部)]

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "character.glb".into());
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system");
    let ag = format!("{root}/AvatarGirl");
    let parts: Vec<String> = [
        "girl_head.fskin",
        "girl_Hair01.fskin",
        "girl_Upper.fskin",
        "girl_Lower.fskin",
        "girl_Boot.fskin",
    ]
    .iter()
    .map(|n| format!("{ag}/{n}"))
    .collect();
    let hat = format!("{root}/Hat/FM01.FSM");
    let skel = format!("{ag}/act/girl.FSKEL");
    let summary = gongfu_studio_lib::export_full_glb_auto(
        &parts,
        Some(&skel),
        Some((&hat, "", [0.0, 0.0, 0.0])),
        &out,
    )
    .unwrap();
    println!("{summary}");
}
