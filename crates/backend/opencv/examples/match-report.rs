//! Prints what this host's OpenCV finds in the matching fixtures, as JSON.
//!
//! Cross-target verification is hands-on: the same fixtures are searched on
//! `aarch64-apple-darwin` and on `x86_64-pc-windows-msvc`, and the two reports
//! are compared. This program is what produces a report, so both runs measure
//! literally the same inputs as the test suite — the scene, the templates, and
//! the requests all come from `mado-pilot-testkit`'s fixture generator, which the
//! tests use unchanged.
//!
//! Run it with a label naming the host, because a score is only evidence when it
//! is attributable:
//!
//! ```text
//! cargo run --release --locked --package mado-pilot-backend-opencv \
//!     --example match-report -- --label "Windows 11 Pro 25H2, Core i7-12700KF"
//! ```
//!
//! Nothing here detects the CPU model or the operating-system build. Those are
//! the operator's to state, because a program that guesses them records a guess.

use std::sync::Arc;

use mado_pilot_capture::PixelFormat;
use mado_pilot_core::{ClipPolicy, CoordinateSpace, OperationContext, PixelExtent, Rect};
use mado_pilot_testkit::match_fixtures;
use mado_pilot_vision::{
    MatchBackend, MatchDefaults, MatchOptions, MatchRequest, MatchResult, Matcher, RegionSelection,
};

use mado_pilot_backend_opencv::OpenCvBackend;

fn main() {
    let label = label().unwrap_or_else(|| "unlabelled".to_owned());
    let backend = OpenCvBackend::new().expect("an OpenCV 4 development installation");
    let descriptor = backend.descriptor();
    let matcher = Matcher::new(Arc::new(backend) as Arc<dyn MatchBackend>);

    println!("{{");
    println!("  \"label\": {},", quote(&label));
    println!("  \"arch\": {},", quote(std::env::consts::ARCH));
    println!("  \"os\": {},", quote(std::env::consts::OS));
    println!("  \"backend\": {},", quote(descriptor.id()));
    println!("  \"opencvVersion\": {},", quote(descriptor.version()));
    println!(
        "  \"pixelFormat\": {},",
        quote(&descriptor.format().to_string())
    );
    println!("  \"fixtures\": [");

    let fixtures = [
        (
            "planted-full-frame-rgba",
            planted(&matcher, PixelFormat::Rgba8, 0.9),
        ),
        (
            "planted-full-frame-bgra",
            planted(&matcher, PixelFormat::Bgra8, 0.9),
        ),
        (
            "planted-with-degraded-copy",
            planted(&matcher, PixelFormat::Rgba8, 0.5),
        ),
        ("planted-region-of-interest", region(&matcher)),
        ("planted-clipped-region", clipped(&matcher)),
        ("absent-best-offset", absent(&matcher)),
        ("oversized-template", oversized(&matcher)),
    ];
    let last = fixtures.len() - 1;
    for (index, (name, result)) in fixtures.into_iter().enumerate() {
        report(name, &result, index == last);
    }

    println!("  ]");
    println!("}}");
}

/// Searches the whole scene, in `format`, for the planted patch.
fn planted(matcher: &Matcher, format: PixelFormat, min_score: f64) -> MatchResult {
    run(
        matcher,
        format,
        RegionSelection::FullFrame,
        min_score,
        match_fixtures::planted_template("patch"),
    )
}

/// Searches a region that holds the second planted copy.
fn region(matcher: &Matcher) -> MatchResult {
    run(
        matcher,
        PixelFormat::Rgba8,
        RegionSelection::Region {
            rect: rect(55.0, 35.0, 80.0, 60.0),
            policy: ClipPolicy::Reject,
        },
        0.9,
        match_fixtures::planted_template("patch"),
    )
}

/// Searches a region that runs past the frame and is clipped to it.
fn clipped(matcher: &Matcher) -> MatchResult {
    run(
        matcher,
        PixelFormat::Rgba8,
        RegionSelection::Region {
            rect: rect(55.0, 35.0, 200.0, 200.0),
            policy: ClipPolicy::Clip,
        },
        0.9,
        match_fixtures::planted_template("patch"),
    )
}

/// Reports the best offset for a patch the scene does not contain, so the margin
/// below a real threshold is recorded rather than assumed.
fn absent(matcher: &Matcher) -> MatchResult {
    run(
        matcher,
        PixelFormat::Rgba8,
        RegionSelection::FullFrame,
        0.0,
        match_fixtures::absent_template("absent"),
    )
}

/// Searches for a template larger than the frame, which finds nothing.
fn oversized(matcher: &Matcher) -> MatchResult {
    run(
        matcher,
        PixelFormat::Rgba8,
        RegionSelection::FullFrame,
        0.5,
        match_fixtures::oversized_template("wide", PixelExtent::new(120, 80)),
    )
}

fn run(
    matcher: &Matcher,
    format: PixelFormat,
    selection: RegionSelection,
    min_score: f64,
    template: mado_pilot_vision::TemplateSource,
) -> MatchResult {
    let frame = match_fixtures::scene_frame(format);
    let prepared = matcher
        .prepare(&template, &OperationContext::new())
        .expect("a fixture template prepares");
    let options =
        MatchOptions::from_defaults(MatchDefaults::new(min_score, 8).expect("valid defaults"));

    matcher
        .find(
            MatchRequest::new(&frame, selection, &prepared, options),
            &OperationContext::new(),
        )
        .expect("a well-formed fixture search")
}

fn report(name: &str, result: &MatchResult, last: bool) {
    let searched = result.searched();
    println!("    {{");
    println!("      \"fixture\": {},", quote(name));
    println!(
        "      \"searched\": [{}, {}, {}, {}],",
        searched.left(),
        searched.top(),
        searched.right(),
        searched.bottom()
    );
    println!("      \"matches\": [");

    let total = result.matches().len();
    for (index, found) in result.matches().iter().enumerate() {
        let bounds = found.bounds();
        let comma = if index + 1 == total { "" } else { "," };
        println!(
            "        {{ \"template\": {}, \"bounds\": [{}, {}, {}, {}], \"score\": {:.17} }}{comma}",
            quote(found.template().as_str()),
            bounds.left(),
            bounds.top(),
            bounds.right(),
            bounds.bottom(),
            found.score()
        );
    }

    println!("      ]");
    println!("    }}{}", if last { "" } else { "," });
}

fn rect(left: f64, top: f64, right: f64, bottom: f64) -> Rect {
    Rect::new(CoordinateSpace::CapturePixels, left, top, right, bottom).expect("a valid rectangle")
}

/// Returns the value of a `--label` argument, if one was supplied.
fn label() -> Option<String> {
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--label" {
            return arguments.next();
        }
        if let Some(value) = argument.strip_prefix("--label=") {
            return Some(value.to_owned());
        }
    }

    None
}

/// Quotes a string as JSON, escaping what the format requires.
fn quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            control if control.is_control() => {
                quoted.push_str(&format!("\\u{:04x}", u32::from(control)));
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');

    quoted
}
