//! The concrete clients every belt is built from (in progress).

/// The variables a configuration needs to load, with placeholder values and
/// `DRY_RUN` on: what [`Deps::fake`] runs on, and what a test reads a
/// [`teeny_core::Config`] from.
pub fn fake_vars() -> Vec<(&'static str, &'static str)> {
    vec![
        ("X_BOT_USERNAME", "teenydroid"),
        ("X_API_KEY", "fake"),
        ("X_API_SECRET", "fake"),
        ("X_ACCESS_TOKEN", "fake"),
        ("X_ACCESS_TOKEN_SECRET", "fake"),
        ("CORTEX_TOKEN", "fake"),
        ("LLM_API_KEY", "fake"),
        ("LLM_MODEL", "chat"),
        ("DRY_RUN", "true"),
    ]
}
