//! Opt-in benchmark settings shared by the actor and presentation probe.
/// Bounded synthetic load; normal sessions do not consult this setting.
pub fn noisy_count() -> usize {
    std::env::var("TERVY_PERF_NOISY")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|count| (1..=20).contains(count))
        .unwrap_or(10)
}
