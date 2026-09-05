fn main() {
    connectrpc_build::Config::new()
        .files(&["../../proto/audio/moderation/v1/audio_moderation.proto"])
        .includes(&["../../proto"])
        .include_file("_connectrpc.rs")
        .compile()
        .expect("failed to generate Connect RPC types");
}
