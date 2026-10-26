//! Read-side replay cost: how the per-step "current view" derivation scales with log length.
//!
//! Not a behavioural gate; a measuring stick. Run with:
//! `cargo test --test replay_cost -- --nocapture --test-threads=1`

mod common;

use std::time::Instant;

use common::session_data_fixture::SessionDataFixture;
use litecode::session::estimate::count_text_tokens;
use litecode::types::user_text;

/// ~1.1 KB per row, close to the 7.6 MB / 5 021 rows of the largest local session.
fn payload() -> String {
    "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua ".repeat(8)
}

fn build(fx: &SessionDataFixture, rows: usize) -> String {
    let sid = fx.create("/measure", "agent", None);
    let text = payload();
    let items: Vec<_> = (0..rows).map(|i| user_text(format!("{i:05} {text}"))).collect();
    fx.insert_items(&sid, &items);
    sid
}

#[test]
#[ignore = "measurement, not a gate"]
fn replay_cost_scales_with_log_length() {
    let mut rows_out = Vec::new();
    for rows in [500usize, 5000] {
        let fx = SessionDataFixture::new();
        let sid = build(&fx, rows);

        let warm = fx.data.working_set_blocking(&sid).expect("warm read");
        assert_eq!(warm.len(), rows, "one working row per inserted item");

        let mut samples = Vec::new();
        for _ in 0..7 {
            let t = Instant::now();
            let ws = fx.data.working_set_blocking(&sid).expect("read");
            std::hint::black_box(&ws);
            samples.push(t.elapsed().as_micros());
        }
        samples.sort_unstable();
        let median = samples[samples.len() / 2];

        let total_text = payload().repeat(rows);
        let t = Instant::now();
        let tokens = std::hint::black_box(count_text_tokens(&total_text));
        let tokenize_us = t.elapsed().as_micros();

        rows_out.push((rows, median / 1000, median % 1000, tokenize_us / 1000, tokens));
    }

    println!("rows | replay ms | tokenize ms | tokens");
    for (rows, ms, rem, tok_ms, tokens) in rows_out {
        println!("{rows:>4} | {ms:>7}.{rem:03} | {tok_ms:>9} | {tokens}");
    }
}

#[test]
#[ignore = "measurement, not a gate"]
fn extending_a_view_costs_the_delta_not_the_log() {
    use litecode::session::view::WorkingView;

    let mut out = Vec::new();
    for rows in [500usize, 5000] {
        let fx = SessionDataFixture::new();
        let sid = build(&fx, rows);

        let events = fx.data.events_blocking(&sid).expect("events");
        let t = Instant::now();
        let view = WorkingView::from_events(&events).expect("fold");
        let full_us = t.elapsed().as_micros();

        let extra: Vec<_> = (0..3).map(|i| user_text(format!("extra {i}"))).collect();
        fx.insert_items(&sid, &extra);
        let from = view.max_seq().expect("high-water") + 1;
        let (last, _) = fx.data.seq_cursor_blocking(&sid).expect("cursor");
        let delta = fx
            .data
            .events_range_blocking(&sid, from as i64, last + 1)
            .expect("delta");
        assert_eq!(delta.len(), 3);

        let t = Instant::now();
        let mut extended = view;
        for event in &delta {
            extended.apply(event).expect("apply");
        }
        std::hint::black_box(extended.rows().expect("rows"));
        let delta_us = t.elapsed().as_micros();

        out.push((rows, full_us, delta_us, delta.len()));
    }

    println!("rows | full fold us | extend 3 rows us");
    for (rows, full, delta, n) in out {
        println!("{rows:>4} | {full:>12} | {delta:>17}  ({n} rows)");
    }
}
