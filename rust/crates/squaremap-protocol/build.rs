fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc is available");
    unsafe {
        std::env::set_var("PROTOC", protoc);
    }

    let manifest_dir = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repository_root = manifest_dir.join("../../..");
    let proto_root = repository_root.join("protocol");
    let proto = proto_root.join("squaremap/bridge/v1/bridge.proto");

    println!("cargo:rerun-if-changed={}", proto.display());
    prost_build::Config::new()
        .compile_protos(&[proto], &[proto_root])
        .expect("bridge.proto compiles");
}
