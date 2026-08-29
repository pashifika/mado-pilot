//! Waits for stable template presence on one explicitly selected native target.
//!
//! This example never requests permissions, activates a target, injects input,
//! or polls frames. Grant capture permission to the host application before
//! running it where the platform requires that grant.
//!
//! ```text
//! cargo run --locked --package mado-pilot --example native-template-watch -- \
//!   <asset-package-directory> <template-name> <target-index>
//! ```

use std::path::PathBuf;
use std::time::Duration;

use mado_pilot::{
    Engine, MatchOptions, NativeEngineRequest, OpenRequest, OperationContext, PackageSource,
    TemplateStability, TemplateTerminalOutcome, TemplateWatchRequest,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let package_root = arguments.next().ok_or(
        "usage: native-template-watch <asset-package-directory> <template-name> <target-index>",
    )?;
    let template_name = arguments
        .next()
        .ok_or("missing template name")?
        .into_string()
        .map_err(|_| "template name must be valid UTF-8")?;
    let target_index = arguments
        .next()
        .ok_or("missing target index")?
        .into_string()
        .map_err(|_| "target index must be valid UTF-8")?
        .parse::<usize>()?;
    if arguments.next().is_some() {
        return Err("unexpected argument".into());
    }

    let engine = native_engine()?;
    let setup = OperationContext::new().with_timeout(Duration::from_secs(10))?;
    let targets = engine.discover(&setup)?;
    let target = targets.get(target_index).ok_or_else(|| {
        format!(
            "target index {target_index} is unavailable; discovery returned {} targets",
            targets.len()
        )
    })?;
    let session = engine.open(target.id(), &OpenRequest::new(), &setup)?;
    let package = engine.load_package(
        &PackageSource::directory(PathBuf::from(package_root)),
        &setup,
    )?;
    let template = engine.prepare_from_package(&package, &template_name, &setup)?;

    // Query lifetime and this caller's blocking wait have independent authority.
    let query_operation = OperationContext::new().with_timeout(Duration::from_secs(30))?;
    let query = session.start_template_watch(
        TemplateWatchRequest::new(
            template.clone(),
            MatchOptions::from_defaults(template.defaults()),
            query_operation,
        )
        .with_stability(TemplateStability::consecutive(2)?),
    )?;
    let wait_operation = OperationContext::new().with_timeout(Duration::from_secs(35))?;
    let outcome = query.wait(&wait_operation)?;

    let result = match outcome.as_ref() {
        TemplateTerminalOutcome::Matched(result) => result,
        TemplateTerminalOutcome::Cancelled => return Err("template query was cancelled".into()),
        TemplateTerminalOutcome::DeadlineExceeded => {
            return Err("template query deadline expired".into());
        }
        TemplateTerminalOutcome::SessionClosed => {
            return Err("capture session closed before a stable match".into());
        }
        TemplateTerminalOutcome::SchedulerClosed => {
            return Err("template scheduler closed before a stable match".into());
        }
        TemplateTerminalOutcome::TargetLost => {
            return Err("capture target was lost before a stable match".into());
        }
        TemplateTerminalOutcome::Overloaded(_) => {
            return Err("finite watcher capacity could not satisfy the query".into());
        }
        TemplateTerminalOutcome::Failed(_) => {
            return Err("capture, mapping, or template matching failed".into());
        }
        _ => return Err("template query ended with an unsupported outcome".into()),
    };

    let stamp = result.frame().stamp();
    println!(
        "stable native template: {} match(es), stream {:?}, epoch {}, sequence {}, geometry {}",
        result.result().matches().len(),
        stamp.stream(),
        stamp.epoch().value(),
        stamp.sequence().value(),
        stamp.geometry().value(),
    );
    println!(
        "confirmed observations: {}, searched: {}, backend: {}",
        result.confirmed_observations(),
        result.result().searched(),
        result.result().backend(),
    );

    session.close(&setup)?;
    Ok(())
}

fn native_engine() -> Result<Engine, Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        return Ok(mado_pilot::windows_engine(NativeEngineRequest::new())?);
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(mado_pilot::macos_engine(NativeEngineRequest::new())?);
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Err("native template watching is supported only on Windows and macOS".into())
    }
}
