//! Waits for stable template presence on one explicitly selected native target.
//!
//! This example never requests permissions, activates a target, injects input,
//! or polls frames. Grant capture permission to the host application before
//! running it where the platform requires that grant.
//!
//! The native watcher wiring shown here is implemented but not a production
//! support statement. Native watcher support remains withheld until the same
//! qualification protocol passes on both release hosts.
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
    let outcome = (|| -> Result<_, Box<dyn std::error::Error>> {
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
        Ok(query.wait(&wait_operation)?)
    })();
    let close_operation = OperationContext::new()
        .with_timeout(Duration::from_secs(10))
        .expect("the fixed close timeout is representable");
    let close_result = session.close(&close_operation);
    close_result?;
    let outcome = outcome?;

    let result: Result<_, Box<dyn std::error::Error>> = match outcome.as_ref() {
        TemplateTerminalOutcome::Matched(result) => Ok(result),
        TemplateTerminalOutcome::Cancelled => Err("template query was cancelled".into()),
        TemplateTerminalOutcome::DeadlineExceeded => Err("template query deadline expired".into()),
        TemplateTerminalOutcome::SessionClosed => {
            Err("capture session closed before a stable match".into())
        }
        TemplateTerminalOutcome::SchedulerClosed => {
            Err("template scheduler closed before a stable match".into())
        }
        TemplateTerminalOutcome::TargetLost => {
            Err("capture target was lost before a stable match".into())
        }
        TemplateTerminalOutcome::Overloaded(_) => {
            Err("finite watcher capacity could not satisfy the query".into())
        }
        TemplateTerminalOutcome::Failed(_) => {
            Err("capture, mapping, or template matching failed".into())
        }
        _ => Err("template query ended with an unsupported outcome".into()),
    };
    let result = result?;

    // The matched result remains readable after the maintained session closes.
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

    Ok(())
}

#[cfg(windows)]
fn native_engine() -> Result<Engine, Box<dyn std::error::Error>> {
    Ok(mado_pilot::windows_engine(NativeEngineRequest::new())?)
}

#[cfg(target_os = "macos")]
fn native_engine() -> Result<Engine, Box<dyn std::error::Error>> {
    Ok(mado_pilot::macos_engine(NativeEngineRequest::new())?)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn native_engine() -> Result<Engine, Box<dyn std::error::Error>> {
    Err(
        "native template watcher wiring is available only on Windows and macOS; production support is withheld on all platforms pending qualification"
            .into(),
    )
}
