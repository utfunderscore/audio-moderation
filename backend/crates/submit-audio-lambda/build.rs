fn main() {
    let mut includes = vec![std::path::PathBuf::from("../../../proto")];
    if let Ok(protoc_include) = std::env::var("PROTOC_INCLUDE") {
        includes.push(protoc_include.into());
    }

    connectrpc_build::Config::new()
        .files(&["../../../proto/audio/review/v1/audio_review.proto"])
        .includes(&includes)
        .include_file("_connectrpc.rs")
        .compile()
        .expect("failed to generate Connect RPC types");
}
