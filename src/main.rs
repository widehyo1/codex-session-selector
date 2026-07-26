fn main() -> anyhow::Result<()> {
    codex_session_selector::run_from_args(std::env::args().skip(1))
}
