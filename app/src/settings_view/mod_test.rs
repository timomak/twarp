use super::*;
use settings_page::MatchData;

// ── SettingsSection classification ──────────────────────────────────────────

#[test]
fn code_subpages_are_identified() {
    assert!(SettingsSection::CodeIndexing.is_code_subpage());
    assert!(SettingsSection::EditorAndCodeReview.is_code_subpage());

    assert!(!SettingsSection::Code.is_code_subpage());
    assert!(!SettingsSection::Account.is_code_subpage());
}

#[test]
fn cloud_platform_subpages_are_identified() {
    assert!(SettingsSection::CloudEnvironments.is_cloud_platform_subpage());
    assert!(SettingsSection::OzCloudAPIKeys.is_cloud_platform_subpage());

    assert!(!SettingsSection::Account.is_cloud_platform_subpage());
}

#[test]
fn is_subpage_covers_all_umbrella_types() {
    // All subpages under any umbrella should return true.
    assert!(SettingsSection::CodeIndexing.is_subpage());
    assert!(SettingsSection::EditorAndCodeReview.is_subpage());
    assert!(SettingsSection::CloudEnvironments.is_subpage());
    assert!(SettingsSection::OzCloudAPIKeys.is_subpage());

    // Top-level pages should not be subpages.
    assert!(!SettingsSection::Account.is_subpage());
    assert!(!SettingsSection::Code.is_subpage());
    assert!(!SettingsSection::Privacy.is_subpage());
}

// ── parent_page_section mapping ─────────────────────────────────────────────

#[test]
fn code_subpages_map_to_code_backing_page() {
    assert_eq!(
        SettingsSection::CodeIndexing.parent_page_section(),
        SettingsSection::Code
    );
    assert_eq!(
        SettingsSection::EditorAndCodeReview.parent_page_section(),
        SettingsSection::Code
    );
}

#[test]
fn cloud_platform_subpages_map_to_their_backing_pages() {
    assert_eq!(
        SettingsSection::CloudEnvironments.parent_page_section(),
        SettingsSection::CloudEnvironments
    );
    assert_eq!(
        SettingsSection::OzCloudAPIKeys.parent_page_section(),
        SettingsSection::OzCloudAPIKeys
    );
}

#[test]
fn non_subpage_sections_map_to_themselves() {
    assert_eq!(
        SettingsSection::Account.parent_page_section(),
        SettingsSection::Account
    );
    assert_eq!(
        SettingsSection::Privacy.parent_page_section(),
        SettingsSection::Privacy
    );
}

// ── MatchData behavior ──────────────────────────────────────────────────────

#[test]
fn match_data_uncounted_true_is_truthy() {
    assert!(MatchData::Uncounted(true).is_truthy());
}

#[test]
fn match_data_uncounted_false_is_not_truthy() {
    assert!(!MatchData::Uncounted(false).is_truthy());
}

#[test]
fn match_data_countable_nonzero_is_truthy() {
    assert!(MatchData::Countable(3).is_truthy());
    assert!(MatchData::Countable(1).is_truthy());
}

#[test]
fn match_data_countable_zero_is_not_truthy() {
    assert!(!MatchData::Countable(0).is_truthy());
}

// ── Display / FromStr round-trip ────────────────────────────────────────────

#[test]
fn subpage_display_names_are_correct() {
    assert_eq!(
        SettingsSection::CodeIndexing.to_string(),
        "Indexing and projects"
    );
    assert_eq!(
        SettingsSection::EditorAndCodeReview.to_string(),
        "Editor and Code Review"
    );
    assert_eq!(
        SettingsSection::CloudEnvironments.to_string(),
        "Environments"
    );
    assert_eq!(
        SettingsSection::OzCloudAPIKeys.to_string(),
        "Oz Cloud API Keys"
    );
}

#[test]
fn subpage_from_str_parses_display_names() {
    // Both the legacy "Oz" name and the new "Warp Agent" display name still
    // resolve to SettingsSection::WarpAgent so existing deep links and
    // persisted telemetry strings keep parsing while the AI page is removed.
    assert_eq!(
        SettingsSection::from_str("Oz"),
        Ok(SettingsSection::WarpAgent)
    );
    assert_eq!(
        SettingsSection::from_str("Warp Agent"),
        Ok(SettingsSection::WarpAgent)
    );
    assert_eq!(
        SettingsSection::from_str("Indexing and projects"),
        Ok(SettingsSection::CodeIndexing)
    );
    assert_eq!(
        SettingsSection::from_str("Editor and Code Review"),
        Ok(SettingsSection::EditorAndCodeReview)
    );
    assert_eq!(
        SettingsSection::from_str("Oz Cloud API Keys"),
        Ok(SettingsSection::OzCloudAPIKeys)
    );
}

// ── cycle_pages search filter ────────────────────────────────────────────────
// These tests validate the logic added to cycle_pages() to ensure arrow key
// navigation respects the active search filter.

/// Mirrors the filter predicate used in cycle_pages() when search is active.
fn section_passes_nav_filter(
    section: SettingsSection,
    subpage_filter: &HashMap<SettingsSection, MatchData>,
    pages_filter: &[(SettingsSection, MatchData)],
) -> bool {
    if let Some(md) = subpage_filter.get(&section) {
        md.is_truthy()
    } else {
        let backing = section.parent_page_section();
        pages_filter
            .iter()
            .any(|(s, md)| *s == backing && md.is_truthy())
    }
}

#[test]
fn nav_filter_falls_back_to_pages_filter_for_top_level_pages() {
    // Top-level pages (Account, Appearance, etc.) have no subpage_filter entry.
    // They fall back to pages_filter using parent_page_section() == themselves.
    let subpage_filter: HashMap<SettingsSection, MatchData> = HashMap::new();
    let pages_filter = vec![
        (SettingsSection::Account, MatchData::Uncounted(true)),
        (SettingsSection::Appearance, MatchData::Countable(0)),
        (SettingsSection::Features, MatchData::Uncounted(true)),
    ];

    assert!(section_passes_nav_filter(
        SettingsSection::Account,
        &subpage_filter,
        &pages_filter
    ));
    assert!(!section_passes_nav_filter(
        SettingsSection::Appearance,
        &subpage_filter,
        &pages_filter
    ));
    assert!(section_passes_nav_filter(
        SettingsSection::Features,
        &subpage_filter,
        &pages_filter
    ));
}

// ── Collapsed umbrella nav-stop behavior ────────────────────────────────────
// Verify that arrow-key navigation lands on a collapsed umbrella as a single
// stop (and activates it by jumping to the first subpage, which auto-expands
// the umbrella) instead of silently skipping over it.

use nav::{SettingsNavItem, SettingsUmbrella};

/// Builds the nav-items layout used by `SettingsView::new`, matching the real
/// sidebar ordering so tests exercise realistic nav orders.
fn realistic_nav_items() -> Vec<SettingsNavItem> {
    vec![
        SettingsNavItem::Page(SettingsSection::Account),
        SettingsNavItem::Page(SettingsSection::BillingAndUsage),
        SettingsNavItem::Umbrella(SettingsUmbrella::new(
            "Code",
            SettingsSection::code_subpages().to_vec(),
        )),
        SettingsNavItem::Umbrella(SettingsUmbrella::new(
            "Cloud platform",
            SettingsSection::cloud_platform_subpages().to_vec(),
        )),
        SettingsNavItem::Page(SettingsSection::Teams),
    ]
}

/// Mutably flips an umbrella's `expanded` flag at `nav_index`.
fn set_expanded(nav_items: &mut [SettingsNavItem], nav_index: usize, expanded: bool) {
    if let Some(SettingsNavItem::Umbrella(u)) = nav_items.get_mut(nav_index) {
        u.expanded = expanded;
    } else {
        panic!("nav_items[{nav_index}] is not an Umbrella");
    }
}

#[test]
fn collapsed_umbrella_is_a_single_nav_stop() {
    let nav_items = realistic_nav_items();
    // All umbrellas default to collapsed.
    let stops = build_nav_stops(&nav_items, |_| true);

    // Expect: Account, BillingAndUsage, <Code umbrella>,
    // <Cloud platform umbrella>, Teams.
    assert_eq!(stops.len(), 5);
    assert!(matches!(
        stops[0],
        NavStop::Section(SettingsSection::Account)
    ));
    assert!(matches!(
        stops[1],
        NavStop::Section(SettingsSection::BillingAndUsage)
    ));
    assert!(matches!(
        stops[2],
        NavStop::CollapsedUmbrella {
            nav_index: 2,
            first_subpage: SettingsSection::CodeIndexing,
            last_subpage: SettingsSection::EditorAndCodeReview,
        }
    ));
    assert!(matches!(
        stops[3],
        NavStop::CollapsedUmbrella {
            nav_index: 3,
            first_subpage: SettingsSection::CloudEnvironments,
            last_subpage: SettingsSection::OzCloudAPIKeys,
        }
    ));
    assert!(matches!(stops[4], NavStop::Section(SettingsSection::Teams)));
}

#[test]
fn expanded_umbrella_produces_section_stop_per_subpage() {
    let mut nav_items = realistic_nav_items();
    // Expand the Code umbrella so each of its subpages becomes a nav stop.
    set_expanded(&mut nav_items, 2, true);

    let stops = build_nav_stops(&nav_items, |_| true);

    // Expect: Account, BillingAndUsage, CodeIndexing, EditorAndCodeReview,
    // <Cloud platform umbrella>, Teams.
    let sections: Vec<_> = stops
        .iter()
        .map(|s| match s {
            NavStop::Section(section) => format!("{section:?}"),
            NavStop::CollapsedUmbrella { nav_index, .. } => format!("Umbrella@{nav_index}"),
        })
        .collect();
    assert_eq!(
        sections,
        vec![
            "Account",
            "BillingAndUsage",
            "CodeIndexing",
            "EditorAndCodeReview",
            "Umbrella@3",
            "Teams",
        ]
    );
}

#[test]
fn collapsed_umbrella_with_filtered_subpages_uses_first_visible_subpage() {
    // When a search filter hides the first subpage, activating the collapsed
    // umbrella should land on the *next* visible subpage (still auto-expanding).
    let nav_items = realistic_nav_items();

    let stops = build_nav_stops(&nav_items, |section| {
        // Hide CodeIndexing (first Code subpage); keep the rest.
        section != SettingsSection::CodeIndexing
    });

    let code_stop = stops
        .iter()
        .find(|s| matches!(s, NavStop::CollapsedUmbrella { nav_index: 2, .. }))
        .expect("Code umbrella should still be a collapsed stop");

    match code_stop {
        NavStop::CollapsedUmbrella {
            first_subpage,
            last_subpage,
            ..
        } => {
            assert_eq!(
                *first_subpage,
                SettingsSection::EditorAndCodeReview,
                "CodeIndexing is hidden by the filter, so the first visible subpage is EditorAndCodeReview"
            );
            assert_eq!(
                *last_subpage,
                SettingsSection::EditorAndCodeReview,
                "last_subpage tracks the last visible subpage; only one remains"
            );
        }
        _ => unreachable!(),
    }
}

#[test]
fn umbrella_with_no_visible_subpages_is_skipped_entirely() {
    let nav_items = realistic_nav_items();

    let stops = build_nav_stops(&nav_items, |section| !section.is_code_subpage());

    // The Code umbrella's subpages are all filtered out, so the entire
    // umbrella should be absent from the nav order.
    assert!(
        stops
            .iter()
            .all(|s| !matches!(s, NavStop::CollapsedUmbrella { nav_index: 2, .. })),
        "Code umbrella should not appear when none of its subpages are visible"
    );
    // The still-visible Cloud platform umbrella remains as a stop.
    assert!(stops
        .iter()
        .any(|s| matches!(s, NavStop::CollapsedUmbrella { nav_index: 3, .. })));
}

#[test]
fn filtered_out_top_level_page_is_skipped() {
    let nav_items = realistic_nav_items();

    let stops = build_nav_stops(&nav_items, |section| section != SettingsSection::Teams);

    assert!(
        !stops
            .iter()
            .any(|s| matches!(s, NavStop::Section(SettingsSection::Teams))),
        "Teams should be filtered out entirely"
    );
    // But other pages remain.
    assert!(stops
        .iter()
        .any(|s| matches!(s, NavStop::Section(SettingsSection::Account))));
}

// ── current_stop_index ──────────────────────────────────────────────────────

#[test]
fn current_stop_index_matches_section_stop() {
    let nav_items = realistic_nav_items();
    let stops = build_nav_stops(&nav_items, |_| true);

    let idx = current_stop_index(&stops, &nav_items, SettingsSection::BillingAndUsage);
    assert_eq!(idx, Some(1));
}

#[test]
fn current_stop_index_maps_subpage_to_collapsed_umbrella() {
    // Edge case: the user manually collapsed the Code umbrella while still
    // on one of its subpages. The collapsed umbrella should match as the
    // current stop so arrow-key cycling continues from the umbrella's position.
    let nav_items = realistic_nav_items();
    let stops = build_nav_stops(&nav_items, |_| true);

    let idx = current_stop_index(&stops, &nav_items, SettingsSection::EditorAndCodeReview);
    assert_eq!(
        idx,
        Some(2),
        "EditorAndCodeReview is under the collapsed Code umbrella at nav_index 2"
    );
}

#[test]
fn current_stop_index_returns_none_when_section_is_not_present() {
    let nav_items = realistic_nav_items();
    // Filter out all Code subpages (and therefore the Code umbrella) entirely.
    let stops = build_nav_stops(&nav_items, |section| !section.is_code_subpage());

    // CodeIndexing isn't directly in stops, and no remaining collapsed umbrella
    // contains it, so current_stop_index should return None.
    assert_eq!(
        current_stop_index(&stops, &nav_items, SettingsSection::CodeIndexing),
        None
    );
}

// ── next_stop_index wrapping ────────────────────────────────────────────────

#[test]
fn next_stop_index_wraps_at_ends() {
    assert_eq!(next_stop_index(0, 3, CycleDirection::Up), 2);
    assert_eq!(next_stop_index(2, 3, CycleDirection::Down), 0);
    assert_eq!(next_stop_index(1, 3, CycleDirection::Up), 0);
    assert_eq!(next_stop_index(1, 3, CycleDirection::Down), 2);
}

#[test]
fn next_stop_index_handles_single_stop() {
    assert_eq!(next_stop_index(0, 1, CycleDirection::Up), 0);
    assert_eq!(next_stop_index(0, 1, CycleDirection::Down), 0);
}

// ── End-to-end cycling (no search) ──────────────────────────────────────────
// These tests simulate the sequence of nav-stop activations that would result
// from repeatedly pressing Down/Up, ensuring a collapsed umbrella is never
// skipped over.

/// Computes the section that would become active after applying the direction
/// once, starting from `current`. Mirrors the final target-resolution step in
/// `cycle_pages`.
fn simulate_cycle(
    nav_items: &[SettingsNavItem],
    stops: &[NavStop],
    current: SettingsSection,
    direction: CycleDirection,
) -> SettingsSection {
    let active = current_stop_index(stops, nav_items, current)
        .expect("current should exist in stops in these tests");
    let next = next_stop_index(active, stops.len(), direction);
    match stops[next] {
        NavStop::Section(section) => section,
        NavStop::CollapsedUmbrella {
            first_subpage,
            last_subpage,
            ..
        } => match direction {
            CycleDirection::Up => last_subpage,
            CycleDirection::Down => first_subpage,
        },
    }
}

#[test]
fn arrow_down_from_billing_with_collapsed_code_lands_on_first_subpage() {
    let nav_items = realistic_nav_items();
    let stops = build_nav_stops(&nav_items, |_| true);

    // Pressing Down from BillingAndUsage should auto-expand Code and select
    // CodeIndexing, not skip over to the Cloud platform umbrella.
    let next = simulate_cycle(
        &nav_items,
        &stops,
        SettingsSection::BillingAndUsage,
        CycleDirection::Down,
    );
    assert_eq!(next, SettingsSection::CodeIndexing);
}

#[test]
fn arrow_up_into_collapsed_umbrella_lands_on_last_subpage() {
    let nav_items = realistic_nav_items();
    let stops = build_nav_stops(&nav_items, |_| true);

    // Pressing Up from Teams should land on the collapsed Cloud platform
    // umbrella, which resolves to OzCloudAPIKeys (last visible subpage) so
    // the user continues moving in natural reading order rather than being
    // jumped back to the top of the umbrella.
    let next = simulate_cycle(
        &nav_items,
        &stops,
        SettingsSection::Teams,
        CycleDirection::Up,
    );
    assert_eq!(next, SettingsSection::OzCloudAPIKeys);
}

#[test]
fn arrow_down_from_expanded_last_subpage_leaves_umbrella() {
    let mut nav_items = realistic_nav_items();
    set_expanded(&mut nav_items, 2, true); // expand Code
    let stops = build_nav_stops(&nav_items, |_| true);

    // EditorAndCodeReview is the last Code subpage; Down should move to the
    // Cloud platform umbrella (the next stop in the nav order).
    let next = simulate_cycle(
        &nav_items,
        &stops,
        SettingsSection::EditorAndCodeReview,
        CycleDirection::Down,
    );
    assert_eq!(next, SettingsSection::CloudEnvironments);
}

#[test]
fn arrow_down_across_adjacent_collapsed_umbrellas() {
    let nav_items = realistic_nav_items();
    // Both Code and Cloud platform umbrellas are collapsed.
    let stops = build_nav_stops(&nav_items, |_| true);

    // From BillingAndUsage, Down should land on the first Code subpage
    // (Code umbrella auto-expands).
    let next_after_billing = simulate_cycle(
        &nav_items,
        &stops,
        SettingsSection::BillingAndUsage,
        CycleDirection::Down,
    );
    assert_eq!(next_after_billing, SettingsSection::CodeIndexing);

    // From the Code umbrella stop (i.e. the user is "on" CodeIndexing which
    // maps back to the collapsed umbrella), pressing Down again should land
    // on the Cloud platform umbrella's first subpage.
    let next_after_code = simulate_cycle(
        &nav_items,
        &stops,
        SettingsSection::CodeIndexing,
        CycleDirection::Down,
    );
    assert_eq!(next_after_code, SettingsSection::CloudEnvironments);
}
