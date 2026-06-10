$ErrorActionPreference = "Stop"

cargo test -p provider-openai-compatible real_openai_compatible_streams_text -- --ignored --nocapture
cargo test -p provider-ollama real_ollama_streams_text -- --ignored --nocapture
cargo test -p provider-lmstudio real_lmstudio_streams_text -- --ignored --nocapture
