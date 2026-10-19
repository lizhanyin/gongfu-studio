use std::fs;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = fs::read(&args[1]).unwrap();
    let t = gongfu_studio_lib::dds_decode_for_test(&bytes).unwrap();
    fs::write(&args[2], t).unwrap();
    println!("ok");
}
