//! Throughput of the two detectors.
//!
//! The budget that matters: six players analysed at 100 Hz means 600 detections per second,
//! and the whole thing has to fit alongside decoding and rendering on a Steam Deck.

use criterion::{criterion_group, criterion_main, Criterion};
use rungstar_pitch::{Algorithm, Analyzer, AnalyzerConfig, ANALYSIS_WINDOW};

fn sung_note(frequency: f64, sample_rate: u32) -> Vec<i16> {
    // A few harmonics, because a voice is not a sine wave and the detectors behave
    // differently when there are overtones to be confused by.
    (0..ANALYSIS_WINDOW)
        .map(|n| {
            let t = n as f64 / f64::from(sample_rate);
            let value = (t * frequency * std::f64::consts::TAU).sin() * 0.6
                + (t * frequency * 2.0 * std::f64::consts::TAU).sin() * 0.25
                + (t * frequency * 3.0 * std::f64::consts::TAU).sin() * 0.15;
            (value * 12_000.0) as i16
        })
        .collect()
}

fn bench_detectors(c: &mut Criterion) {
    let samples = sung_note(220.0, 44_100);

    let mut group = c.benchmark_group("detect");
    for algorithm in [Algorithm::Camdf, Algorithm::Mpm] {
        let mut analyzer = Analyzer::new(AnalyzerConfig {
            algorithm,
            ..AnalyzerConfig::default()
        });
        analyzer.push(&samples);
        group.bench_function(format!("{algorithm:?}"), |b| {
            b.iter(|| std::hint::black_box(analyzer.detect()));
        });
    }
    group.finish();
}

fn bench_push(c: &mut Criterion) {
    // One capture block at 44.1 kHz with a 512-sample period.
    let block = sung_note(220.0, 44_100)[..512].to_vec();
    let mut analyzer = Analyzer::new(AnalyzerConfig::default());
    c.bench_function("push_512", |b| {
        b.iter(|| analyzer.push(std::hint::black_box(&block)));
    });
}

criterion_group!(benches, bench_detectors, bench_push);
criterion_main!(benches);
