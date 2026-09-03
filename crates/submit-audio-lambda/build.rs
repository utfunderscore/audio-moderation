fn main() {
    connectrpc_build::Config::new()
        .files(&["../../proto/audio/review/v1/audio_review.proto"])
        .includes(&["../../proto"])
        .include_file("_connectrpc.rs")
        .compile()
        .expect("failed to generate Connect RPC types");
}
