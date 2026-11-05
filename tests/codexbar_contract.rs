use cosmic_ext_applet_codexbar::{parse_cost_json, parse_usage_json};

#[test]
fn reads_v083_usage_with_new_optional_metadata_and_a_provider_error() {
    let payloads = parse_usage_json(include_str!("fixtures/codexbar-v0.83.0/usage.json"))
        .expect("reviewed v0.83.0 usage contract");
    assert_eq!(payloads.len(), 2);
    let codex = &payloads[0];
    assert_eq!(codex.provider, "codex");
    assert_eq!(codex.account_text(), Some("personal@example.com"));
    assert_eq!(
        codex.window_labels(),
        [Some("Session"), Some("Weekly"), None]
    );
    let usage = codex.usage.as_ref().unwrap();
    let primary = usage.primary.as_ref().unwrap();
    assert_eq!(primary.fraction(), Some(0.25));
    assert_eq!(
        primary.resets_at.unwrap().to_rfc3339(),
        "2026-11-05T06:00:00+00:00"
    );
    assert_eq!(usage.secondary.as_ref().unwrap().used_percent, Some(50.0));
    assert!(usage.tertiary.is_none());
    let row = &usage.details.as_ref().unwrap()[0].rows.as_ref().unwrap()[0];
    assert_eq!(row.value.as_deref(), Some("250 of 1000"));
    assert_eq!(row.secondary_value.as_deref(), Some("Within allowance"));
    assert_eq!(codex.credits.as_ref().unwrap().remaining, Some(12.5));
    let pace = codex.pace.as_ref().unwrap().primary.as_ref().unwrap();
    assert_eq!(
        pace.projection_text().as_deref(),
        Some("Projected empty in 2h 30m")
    );
    assert_eq!(pace.stage_text().as_deref(), Some("5% in deficit"));

    let claude = &payloads[1];
    assert_eq!(claude.provider, "claude");
    assert!(claude.usage.is_none());
    assert_eq!(claude.error.as_ref().unwrap().message, "Not signed in");
    assert_eq!(
        claude.status.as_ref().unwrap().indicator.as_deref(),
        Some("minor")
    );
}

#[test]
fn reads_v083_cost_totals_alongside_an_unavailable_provider() {
    let payloads = parse_cost_json(include_str!("fixtures/codexbar-v0.83.0/cost.json"))
        .expect("reviewed v0.83.0 cost contract");
    assert_eq!(payloads.len(), 2);
    let codex = &payloads[0];
    assert_eq!(codex.provider, "codex");
    assert_eq!(codex.currency_code.as_deref(), Some("USD"));
    assert_eq!(codex.session_tokens, Some(1000));
    assert_eq!(codex.session_cost_usd, Some(0.25));
    assert_eq!(codex.last30_days_tokens, Some(12000));
    assert_eq!(codex.last30_days_cost_usd, Some(3.5));
    assert!(codex.has_figures());
    assert_eq!(payloads[1].provider, "claude");
    assert!(!payloads[1].has_figures());
}
